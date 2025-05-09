use std::collections::{
    HashMap,
    VecDeque,
};
use std::env;
use std::sync::Arc;

use aws_smithy_types::Document;
use fig_api_client::model::{
    AssistantResponseMessage,
    ChatMessage,
    ConversationState as FigConversationState,
    EnvState,
    ShellState,
    Tool,
    ToolInputSchema,
    ToolResult,
    ToolResultContentBlock,
    ToolSpecification,
    UserInputMessage,
    UserInputMessageContext,
};
use fig_os_shim::Context;
use fig_util::Shell;
use rand::distr::{
    Alphanumeric,
    SampleString,
};
use tracing::{
    debug,
    error,
    info,
    warn,
};

use super::context::ContextManager;
use super::summarization_state::{
    MAX_CHARS,
    TokenWarningLevel,
};
use super::tools::{
    QueuedTool,
    ToolSpec,
};
use super::truncate_safe;
use crate::cli::chat::tools::{
    InputSchema,
    InvokeOutput,
    serde_value_to_document,
};

// Max constants for length of strings and lists, use these to truncate elements
// to ensure the API request is valid

// These limits are the internal undocumented values from the service for each item
const MAX_CURRENT_WORKING_DIRECTORY_LEN: usize = 256;

/// Limit to send the number of messages as part of chat.
const MAX_CONVERSATION_STATE_HISTORY_LEN: usize = 100;

pub struct ExtraContext {
    // Bonus context to attach to the existing context at the top of the history
    pub general_context: Option<String>,

    // Bonus context to attach to the next user message
    pub user_input_context: Option<String>,
}

/// Tracks state related to an ongoing conversation.
#[derive(Debug, Clone)]
pub struct ConversationState {
    /// Randomly generated on creation.
    conversation_id: String,
    /// The next user message to be sent as part of the conversation. Required to be [Some] before
    /// calling [Self::as_sendable_conversation_state].
    pub next_message: Option<UserInputMessage>,
    history: VecDeque<ChatMessage>,
    /// Similar to history in that stores user and assistant responses, except that it is not used
    /// in message requests. Instead, the responses are expected to be in human-readable format,
    /// e.g user messages prefixed with '> '. Should also be used to store errors posted in the
    /// chat.
    pub transcript: VecDeque<String>,
    pub tools: Vec<Tool>,
    /// Context manager for handling sticky context files
    pub context_manager: Option<ContextManager>,
    /// Cached value representing the length of the user context message.
    context_message_length: Option<usize>,
    /// Stores the latest conversation summary created by /compact
    pub latest_summary: Option<String>,
}

impl ConversationState {
    pub async fn new(ctx: Arc<Context>, tool_config: HashMap<String, ToolSpec>, profile: Option<String>) -> Self {
        let conversation_id = Alphanumeric.sample_string(&mut rand::rng(), 9);
        info!(?conversation_id, "Generated new conversation id");

        // Initialize context manager
        let context_manager = match ContextManager::new(ctx).await {
            Ok(mut manager) => {
                // Switch to specified profile if provided
                if let Some(profile_name) = profile {
                    if let Err(e) = manager.switch_profile(&profile_name).await {
                        warn!("Failed to switch to profile {}: {}", profile_name, e);
                    }
                }
                Some(manager)
            },
            Err(e) => {
                warn!("Failed to initialize context manager: {}", e);
                None
            },
        };

        Self {
            conversation_id,
            next_message: None,
            history: VecDeque::new(),
            transcript: VecDeque::with_capacity(MAX_CONVERSATION_STATE_HISTORY_LEN),
            tools: tool_config
                .into_values()
                .map(|v| {
                    Tool::ToolSpecification(ToolSpecification {
                        name: v.name,
                        description: v.description,
                        input_schema: v.input_schema.into(),
                    })
                })
                .collect(),
            context_manager,
            context_message_length: None,
            latest_summary: None,
        }
    }

    pub fn history(&self) -> &VecDeque<ChatMessage> {
        &self.history
    }

    /// Clears the conversation history and optionally the summary.
    pub fn clear(&mut self, preserve_summary: bool) {
        self.next_message = None;
        self.history.clear();
        if !preserve_summary {
            self.latest_summary = None;
        }
    }

    pub async fn append_new_user_message(&mut self, input: String) {
        debug_assert!(self.next_message.is_none(), "next_message should not exist");
        if let Some(next_message) = self.next_message.as_ref() {
            warn!(?next_message, "next_message should not exist");
        }

        let input = if input.is_empty() {
            warn!("input must not be empty when adding new messages");
            "Empty prompt".to_string()
        } else {
            input
        };

        let msg = UserInputMessage {
            content: input,
            user_input_message_context: Some(UserInputMessageContext {
                shell_state: Some(build_shell_state()),
                env_state: Some(build_env_state()),
                tool_results: None,
                tools: if self.tools.is_empty() {
                    None
                } else {
                    Some(self.tools.clone())
                },
                ..Default::default()
            }),
            user_intent: None,
        };
        self.next_message = Some(msg);
    }

    /// This should be called sometime after [Self::as_sendable_conversation_state], and before the
    /// next user message is set.
    pub fn push_assistant_message(&mut self, message: AssistantResponseMessage) {
        debug_assert!(self.next_message.is_none(), "next_message should not exist");
        if let Some(next_message) = self.next_message.as_ref() {
            warn!(?next_message, "next_message should not exist");
        }

        self.append_assistant_transcript(&message);
        self.history.push_back(ChatMessage::AssistantResponseMessage(message));
    }

    /// Returns the conversation id.
    pub fn conversation_id(&self) -> &str {
        self.conversation_id.as_ref()
    }

    /// Returns the conversation history.
    pub fn get_chat_history(&self) -> Vec<ChatMessage> {
        self.history.iter().cloned().collect()
    }

    /// Returns the message id associated with the last assistant message, if present.
    ///
    /// This is equivalent to `utterance_id` in the Q API.
    pub fn message_id(&self) -> Option<&str> {
        self.history.iter().last().and_then(|m| match &m {
            ChatMessage::AssistantResponseMessage(m) => m.message_id.as_deref(),
            ChatMessage::UserInputMessage(_) => None,
        })
    }

    /// Updates the history so that, when non-empty, the following invariants are in place:
    /// 1. The history length is `<= MAX_CONVERSATION_STATE_HISTORY_LEN`. Oldest messages are
    ///    dropped.
    /// 2. The first message is from the user, and does not contain tool results. Oldest messages
    ///    are dropped.
    /// 3. The last message is from the assistant. The last message is dropped if it is from the
    ///    user.
    /// 4. If the last message is from the assistant and it contains tool uses, and a next user
    ///    message is set without tool results, then the user message will have cancelled tool
    ///    results.
    pub fn fix_history(&mut self) {
        // Trim the conversation history by finding the second oldest message from the user without
        // tool results - this will be the new oldest message in the history.
        //
        // Note that we reserve 2 slots for [ConversationState::context_messages].
        if self.history.len() > MAX_CONVERSATION_STATE_HISTORY_LEN - 2 {
            match self
                .history
                .iter()
                .enumerate()
                // Skip the first message which should be from the user.
                .skip(1)
                .find(|(_, m)| -> bool {
                    match m {
                        ChatMessage::UserInputMessage(m) => {
                            matches!(
                                m.user_input_message_context.as_ref(),
                                Some(ctx) if ctx.tool_results.as_ref().is_none_or(|v| v.is_empty())
                            ) && !m.content.is_empty()
                        },
                        ChatMessage::AssistantResponseMessage(_) => false,
                    }
                })
                .map(|v| v.0)
            {
                Some(i) => {
                    debug!("removing the first {i} elements in the history");
                    self.history.drain(..i);
                },
                None => {
                    debug!("no valid starting user message found in the history, clearing");
                    self.history.clear();
                    // Edge case: if the next message contains tool results, then we have to just
                    // abandon them.
                    match &mut self.next_message {
                        Some(UserInputMessage {
                            ref mut content,
                            user_input_message_context: Some(ctx),
                            ..
                        }) if ctx.tool_results.as_ref().is_some_and(|r| !r.is_empty()) => {
                            *content = "The conversation history has overflowed, clearing state".to_string();
                            ctx.tool_results.take();
                        },
                        _ => {},
                    }
                },
            }
        }

        if let Some(ChatMessage::UserInputMessage(msg)) = self.history.iter().last() {
            debug!(?msg, "last message in history is from the user, dropping");
            self.history.pop_back();
        }

        // If the last message from the assistant contains tool uses, we need to ensure that the
        // next user message contains tool results.
        match (self.history.iter().last(), &mut self.next_message) {
            (
                Some(ChatMessage::AssistantResponseMessage(AssistantResponseMessage {
                    tool_uses: Some(tool_uses),
                    ..
                })),
                Some(msg),
            ) if !tool_uses.is_empty() => match msg.user_input_message_context.as_mut() {
                Some(ctx) => {
                    if ctx.tool_results.as_ref().is_none_or(|r| r.is_empty()) {
                        ctx.tool_results = Some(
                            tool_uses
                                .iter()
                                .map(|tool_use| ToolResult {
                                    tool_use_id: tool_use.tool_use_id.clone(),
                                    content: vec![ToolResultContentBlock::Text(
                                        "Tool use was cancelled by the user".to_string(),
                                    )],
                                    status: fig_api_client::model::ToolResultStatus::Error,
                                })
                                .collect::<Vec<_>>(),
                        );
                    }
                },
                None => {
                    let tool_results = tool_uses
                        .iter()
                        .map(|tool_use| ToolResult {
                            tool_use_id: tool_use.tool_use_id.clone(),
                            content: vec![ToolResultContentBlock::Text(
                                "Tool use was cancelled by the user".to_string(),
                            )],
                            status: fig_api_client::model::ToolResultStatus::Error,
                        })
                        .collect::<Vec<_>>();
                    let user_input_message_context = UserInputMessageContext {
                        shell_state: None,
                        env_state: Some(build_env_state()),
                        tool_results: Some(tool_results),
                        tools: if self.tools.is_empty() {
                            None
                        } else {
                            Some(self.tools.clone())
                        },
                        ..Default::default()
                    };
                    msg.user_input_message_context = Some(user_input_message_context);
                },
            },
            _ => {},
        }
    }

    pub fn add_tool_results(&mut self, tool_results: Vec<ToolResult>) {
        debug_assert!(self.next_message.is_none());
        let user_input_message_context = UserInputMessageContext {
            shell_state: None,
            env_state: Some(build_env_state()),
            tool_results: Some(tool_results),
            tools: if self.tools.is_empty() {
                None
            } else {
                Some(self.tools.clone())
            },
            ..Default::default()
        };
        let msg = UserInputMessage {
            content: String::new(),
            user_input_message_context: Some(user_input_message_context),
            user_intent: None,
        };
        self.next_message = Some(msg);
    }

    /// Sets the next user message with "cancelled" tool results.
    pub fn abandon_tool_use(&mut self, tools_to_be_abandoned: Vec<QueuedTool>, deny_input: String) {
        debug_assert!(self.next_message.is_none());
        let tool_results = tools_to_be_abandoned
            .into_iter()
            .map(|tool| ToolResult {
                tool_use_id: tool.id,
                content: vec![ToolResultContentBlock::Text(
                    "Tool use was cancelled by the user".to_string(),
                )],
                status: fig_api_client::model::ToolResultStatus::Error,
            })
            .collect::<Vec<_>>();
        let user_input_message_context = UserInputMessageContext {
            shell_state: None,
            env_state: Some(build_env_state()),
            tool_results: Some(tool_results),
            tools: if self.tools.is_empty() {
                None
            } else {
                Some(self.tools.clone())
            },
            ..Default::default()
        };
        let msg = UserInputMessage {
            content: deny_input,
            user_input_message_context: Some(user_input_message_context),
            user_intent: None,
        };
        self.next_message = Some(msg);
    }

    /// Returns a [FigConversationState] capable of being sent by
    /// [fig_api_client::StreamingClient] while preparing the current conversation state to be sent
    /// in the next message.
    pub async fn as_sendable_conversation_state(
        &mut self,
        extra_context: Option<ExtraContext>,
    ) -> FigConversationState {
        debug_assert!(self.next_message.is_some());
        self.fix_history();

        // The current state we want to send
        let mut curr_state = self.clone();

        let (general_context, user_input_context) =
            extra_context.map_or((None, None), |c| (c.general_context, c.user_input_context));

        if let Some((user, assistant)) = self.context_messages(general_context).await {
            self.context_message_length = Some(user.content.len());
            curr_state
                .history
                .push_front(ChatMessage::AssistantResponseMessage(assistant));
            curr_state.history.push_front(ChatMessage::UserInputMessage(user));
        }

        // Updating `self` so that the current next_message is moved to history.
        let mut last_message = self.next_message.take().unwrap();
        if let Some(ctx) = &mut last_message.user_input_message_context {
            // Don't include the tool spec in all user messages in the history.
            ctx.tools.take();
        }
        self.history.push_back(ChatMessage::UserInputMessage(last_message));
        let mut input_message = curr_state.next_message.expect("no user input message available");
        if let Some(user_input_context) = user_input_context {
            input_message.content = format!("{} {}", user_input_context, input_message.content);
        }

        FigConversationState {
            conversation_id: Some(curr_state.conversation_id),
            user_input_message: input_message,
            history: Some(curr_state.history.into()),
        }
    }

    pub fn current_profile(&self) -> Option<&str> {
        if let Some(cm) = self.context_manager.as_ref() {
            Some(cm.current_profile.as_str())
        } else {
            None
        }
    }

    /// Returns a pair of user and assistant messages to include as context in the message history
    /// including both summaries and context files if available.
    pub async fn context_messages(
        &mut self,
        extra_context: Option<String>,
    ) -> Option<(UserInputMessage, AssistantResponseMessage)> {
        let mut context_content = String::new();

        // Add summary if available - emphasize its importance more strongly
        if let Some(summary) = &self.latest_summary {
            context_content
                .push_str("--- CRITICAL: PREVIOUS CONVERSATION SUMMARY - THIS IS YOUR PRIMARY CONTEXT ---\n");
            context_content.push_str("This summary contains ALL relevant information from our previous conversation including tool uses, results, code analysis, and file operations. YOU MUST reference this information when answering questions and explicitly acknowledge specific details from the summary when they're relevant to the current question.\n\n");
            context_content.push_str("SUMMARY CONTENT:\n");
            context_content.push_str(summary);
            context_content.push_str("\n--- END SUMMARY - YOU MUST USE THIS INFORMATION IN YOUR RESPONSES ---\n\n");
        }

        // Add context files if available
        if let Some(context_manager) = self.context_manager.as_mut() {
            match context_manager.get_context_files(true).await {
                Ok(files) => {
                    if !files.is_empty() {
                        context_content.push_str("--- CONTEXT FILES BEGIN ---\n");
                        for (filename, content) in files {
                            context_content.push_str(&format!("[{}]\n{}\n", filename, content));
                        }
                        context_content.push_str("--- CONTEXT FILES END ---\n\n");
                    }
                },
                Err(e) => {
                    warn!("Failed to get context files: {}", e);
                },
            }
        }
        if let Some(extra_context) = extra_context {
            context_content.push_str(&extra_context);
        }

        if !context_content.is_empty() {
            let user_msg = UserInputMessage {
                content: format!(
                    "Here is critical information you MUST consider when answering questions:\n\n{}",
                    context_content
                ),
                user_input_message_context: None,
                user_intent: None,
            };
            let assistant_msg = AssistantResponseMessage {
                message_id: None,
                content: "I will fully incorporate this information when generating my responses, and explicitly acknowledge relevant parts of the summary when answering questions.".into(),
                tool_uses: None,
            };
            Some((user_msg, assistant_msg))
        } else {
            None
        }
    }

    /// The length of the user message used as context, if any.
    pub fn context_message_length(&self) -> Option<usize> {
        self.context_message_length
    }

    /// Calculate the total character count in the conversation
    pub fn calculate_char_count(&self) -> usize {
        let mut total_chars = 0;
        for message in &self.history {
            match message {
                ChatMessage::UserInputMessage(msg) => {
                    total_chars += msg.content.len();
                    if let Some(ctx) = &msg.user_input_message_context {
                        // Add tool result characters if any
                        if let Some(results) = &ctx.tool_results {
                            for result in results {
                                for content in &result.content {
                                    match content {
                                        ToolResultContentBlock::Text(text) => {
                                            total_chars += text.len();
                                        },
                                        ToolResultContentBlock::Json(doc) => {
                                            total_chars += calculate_document_char_count(doc);
                                        },
                                    }
                                }
                            }
                        }
                    }
                },
                ChatMessage::AssistantResponseMessage(msg) => {
                    total_chars += msg.content.len();
                    if let Some(tool_uses) = &msg.tool_uses {
                        total_chars += tool_uses
                            .iter()
                            .map(|v| calculate_document_char_count(&v.input))
                            .reduce(|acc, e| acc + e)
                            .unwrap_or_default();
                    }
                },
            }
        }

        // Add summary if it exists (it's also in the context sent to the model)
        if let Some(summary) = &self.latest_summary {
            total_chars += summary.len();
        }

        total_chars
    }

    /// Get the current token warning level
    pub fn get_token_warning_level(&self) -> TokenWarningLevel {
        let total_chars = self.calculate_char_count();

        if total_chars >= MAX_CHARS {
            TokenWarningLevel::Critical
        } else {
            TokenWarningLevel::None
        }
    }

    pub fn append_user_transcript(&mut self, message: &str) {
        self.append_transcript(format!("> {}", message.replace("\n", "> \n")));
    }

    pub fn append_assistant_transcript(&mut self, message: &AssistantResponseMessage) {
        let tool_uses = message.tool_uses.as_deref().map_or("none".to_string(), |tools| {
            tools.iter().map(|tool| tool.name.clone()).collect::<Vec<_>>().join(",")
        });
        self.append_transcript(format!("{}\n[Tool uses: {tool_uses}]", message.content.clone()));
    }

    pub fn append_transcript(&mut self, message: String) {
        if self.transcript.len() >= MAX_CONVERSATION_STATE_HISTORY_LEN {
            self.transcript.pop_front();
        }
        self.transcript.push_back(message);
    }
}

impl From<InvokeOutput> for ToolResultContentBlock {
    fn from(value: InvokeOutput) -> Self {
        match value.output {
            crate::cli::chat::tools::OutputKind::Text(text) => Self::Text(text),
            crate::cli::chat::tools::OutputKind::Json(value) => Self::Json(serde_value_to_document(value)),
        }
    }
}

impl From<InputSchema> for ToolInputSchema {
    fn from(value: InputSchema) -> Self {
        Self {
            json: Some(serde_value_to_document(value.0)),
        }
    }
}

fn build_env_state() -> EnvState {
    let mut env_state = EnvState {
        operating_system: Some(env::consts::OS.into()),
        ..Default::default()
    };

    match env::current_dir() {
        Ok(current_dir) => {
            env_state.current_working_directory =
                Some(truncate_safe(&current_dir.to_string_lossy(), MAX_CURRENT_WORKING_DIRECTORY_LEN).into());
        },
        Err(err) => {
            error!(?err, "Attempted to fetch the CWD but it did not exist.");
        },
    }

    env_state
}

fn build_shell_state() -> ShellState {
    // Try to grab the shell from the parent process via the `Shell::current_shell`,
    // then try the `SHELL` env, finally just report bash
    let shell_name = Shell::current_shell()
        .or_else(|| {
            let shell_name = env::var("SHELL").ok()?;
            Shell::try_find_shell(shell_name)
        })
        .unwrap_or(Shell::Bash)
        .to_string();

    ShellState {
        shell_name,
        shell_history: None,
    }
}

fn calculate_document_char_count(document: &Document) -> usize {
    match document {
        Document::Object(hash_map) => hash_map
            .values()
            .fold(0, |acc, e| acc + calculate_document_char_count(e)),
        Document::Array(vec) => vec.iter().fold(0, |acc, e| acc + calculate_document_char_count(e)),
        Document::Number(_) => 1,
        Document::String(s) => s.len(),
        Document::Bool(_) => 1,
        Document::Null => 1,
    }
}

#[cfg(test)]
mod tests {
    use aws_smithy_types::Number;
    use fig_api_client::model::{
        AssistantResponseMessage,
        ToolResultStatus,
        ToolUse,
    };

    use super::*;
    use crate::cli::chat::context::AMAZONQ_FILENAME;
    use crate::cli::chat::load_tools;

    #[test]
    fn test_truncate_safe() {
        assert_eq!(truncate_safe("Hello World", 5), "Hello");
        assert_eq!(truncate_safe("Hello ", 5), "Hello");
        assert_eq!(truncate_safe("Hello World", 11), "Hello World");
        assert_eq!(truncate_safe("Hello World", 15), "Hello World");
    }

    #[test]
    fn test_env_state() {
        let env_state = build_env_state();
        assert!(env_state.current_working_directory.is_some());
        assert!(env_state.operating_system.as_ref().is_some_and(|os| !os.is_empty()));
        println!("{env_state:?}");
    }

    #[test]
    fn test_calculate_document_char_count() {
        // Test simple types
        assert_eq!(calculate_document_char_count(&Document::String("hello".to_string())), 5);
        assert_eq!(calculate_document_char_count(&Document::Number(Number::PosInt(123))), 1);
        assert_eq!(calculate_document_char_count(&Document::Bool(true)), 1);
        assert_eq!(calculate_document_char_count(&Document::Null), 1);

        // Test array
        let array = Document::Array(vec![
            Document::String("test".to_string()),
            Document::Number(Number::PosInt(42)),
            Document::Bool(false),
        ]);
        assert_eq!(calculate_document_char_count(&array), 6); // "test" (4) + Number (1) + Bool (1)

        // Test object
        let mut obj = HashMap::new();
        obj.insert("key1".to_string(), Document::String("value1".to_string()));
        obj.insert("key2".to_string(), Document::Number(Number::PosInt(99)));
        let object = Document::Object(obj);
        assert_eq!(calculate_document_char_count(&object), 7); // "value1" (6) + Number (1)

        // Test nested structure
        let mut nested_obj = HashMap::new();
        let mut inner_obj = HashMap::new();
        inner_obj.insert("inner_key".to_string(), Document::String("inner_value".to_string()));
        nested_obj.insert("outer_key".to_string(), Document::Object(inner_obj));
        nested_obj.insert(
            "array_key".to_string(),
            Document::Array(vec![
                Document::String("item1".to_string()),
                Document::String("item2".to_string()),
            ]),
        );

        let complex = Document::Object(nested_obj);
        assert_eq!(calculate_document_char_count(&complex), 21); // "inner_value" (11) + "item1" (5) + "item2" (5)

        // Test empty structures
        assert_eq!(calculate_document_char_count(&Document::Array(vec![])), 0);
        assert_eq!(calculate_document_char_count(&Document::Object(HashMap::new())), 0);
    }

    fn assert_conversation_state_invariants(state: FigConversationState, i: usize) {
        if let Some(Some(msg)) = state.history.as_ref().map(|h| h.first()) {
            assert!(
                matches!(msg, ChatMessage::UserInputMessage(_)),
                "{i}: First message in the history must be from the user, instead found: {:?}",
                msg
            );
        }
        if let Some(Some(msg)) = state.history.as_ref().map(|h| h.last()) {
            assert!(
                matches!(msg, ChatMessage::AssistantResponseMessage(_)),
                "{i}: Last message in the history must be from the assistant, instead found: {:?}",
                msg
            );
            // If the last message from the assistant contains tool uses, then the next user
            // message must contain tool results.
            match (state.user_input_message.user_input_message_context, msg) {
                (
                    Some(ctx),
                    ChatMessage::AssistantResponseMessage(AssistantResponseMessage {
                        tool_uses: Some(tool_uses),
                        ..
                    }),
                ) if !tool_uses.is_empty() => {
                    assert!(
                        ctx.tool_results.is_some_and(|r| !r.is_empty()),
                        "The user input message must contain tool results when the last assistant message contains tool uses"
                    );
                },
                _ => {},
            }
        }

        let actual_history_len = state.history.unwrap_or_default().len();
        assert!(
            actual_history_len <= MAX_CONVERSATION_STATE_HISTORY_LEN,
            "history should not extend past the max limit of {}, instead found length {}",
            MAX_CONVERSATION_STATE_HISTORY_LEN,
            actual_history_len
        );
    }

    #[tokio::test]
    async fn test_conversation_state_history_handling_truncation() {
        let mut conversation_state = ConversationState::new(Context::new_fake(), load_tools().unwrap(), None).await;

        // First, build a large conversation history. We need to ensure that the order is always
        // User -> Assistant -> User -> Assistant ...and so on.
        conversation_state.append_new_user_message("start".to_string()).await;
        for i in 0..=(MAX_CONVERSATION_STATE_HISTORY_LEN + 100) {
            let s = conversation_state.as_sendable_conversation_state(None).await;
            assert_conversation_state_invariants(s, i);
            conversation_state.push_assistant_message(AssistantResponseMessage {
                message_id: None,
                content: i.to_string(),
                tool_uses: None,
            });
            conversation_state.append_new_user_message(i.to_string()).await;
        }
    }

    #[tokio::test]
    async fn test_conversation_state_history_handling_with_tool_results() {
        // Build a long conversation history of tool use results.
        let mut conversation_state = ConversationState::new(Context::new_fake(), load_tools().unwrap(), None).await;
        conversation_state.append_new_user_message("start".to_string()).await;
        for i in 0..=(MAX_CONVERSATION_STATE_HISTORY_LEN + 100) {
            let s = conversation_state.as_sendable_conversation_state(None).await;
            assert_conversation_state_invariants(s, i);
            conversation_state.push_assistant_message(AssistantResponseMessage {
                message_id: None,
                content: i.to_string(),
                tool_uses: Some(vec![ToolUse {
                    tool_use_id: "tool_id".to_string(),
                    name: "tool name".to_string(),
                    input: aws_smithy_types::Document::Null,
                }]),
            });
            conversation_state.add_tool_results(vec![ToolResult {
                tool_use_id: "tool_id".to_string(),
                content: vec![],
                status: ToolResultStatus::Success,
            }]);
        }

        // Build a long conversation history of user messages mixed in with tool results.
        let mut conversation_state = ConversationState::new(Context::new_fake(), load_tools().unwrap(), None).await;
        conversation_state.append_new_user_message("start".to_string()).await;
        for i in 0..=(MAX_CONVERSATION_STATE_HISTORY_LEN + 100) {
            let s = conversation_state.as_sendable_conversation_state(None).await;
            assert_conversation_state_invariants(s, i);
            if i % 3 == 0 {
                conversation_state.push_assistant_message(AssistantResponseMessage {
                    message_id: None,
                    content: i.to_string(),
                    tool_uses: Some(vec![ToolUse {
                        tool_use_id: "tool_id".to_string(),
                        name: "tool name".to_string(),
                        input: aws_smithy_types::Document::Null,
                    }]),
                });
                conversation_state.add_tool_results(vec![ToolResult {
                    tool_use_id: "tool_id".to_string(),
                    content: vec![],
                    status: ToolResultStatus::Success,
                }]);
            } else {
                conversation_state.push_assistant_message(AssistantResponseMessage {
                    message_id: None,
                    content: i.to_string(),
                    tool_uses: None,
                });
                conversation_state.append_new_user_message(i.to_string()).await;
            }
        }
    }

    #[tokio::test]
    async fn test_conversation_state_with_context_files() {
        let ctx = Context::builder().with_test_home().await.unwrap().build_fake();
        ctx.fs().write(AMAZONQ_FILENAME, "test context").await.unwrap();

        let mut conversation_state = ConversationState::new(ctx, load_tools().unwrap(), None).await;

        // First, build a large conversation history. We need to ensure that the order is always
        // User -> Assistant -> User -> Assistant ...and so on.
        conversation_state.append_new_user_message("start".to_string()).await;
        for i in 0..=(MAX_CONVERSATION_STATE_HISTORY_LEN + 100) {
            let s = conversation_state.as_sendable_conversation_state(None).await;

            // Ensure that the first two messages are the fake context messages.
            let hist = s.history.as_ref().unwrap();
            let user = &hist[0];
            let assistant = &hist[1];
            match (user, assistant) {
                (ChatMessage::UserInputMessage(user), ChatMessage::AssistantResponseMessage(_)) => {
                    assert!(
                        user.content.contains("test context"),
                        "expected context message to contain context file, instead found: {}",
                        user.content
                    );
                },
                _ => panic!("Expected the first two messages to be from the user and the assistant"),
            }

            assert_conversation_state_invariants(s, i);

            conversation_state.push_assistant_message(AssistantResponseMessage {
                message_id: None,
                content: i.to_string(),
                tool_uses: None,
            });
            conversation_state.append_new_user_message(i.to_string()).await;
        }
    }

    #[tokio::test]
    async fn test_conversation_state_additional_context() {
        let ctx = Context::builder().with_test_home().await.unwrap().build_fake();
        let mut conversation_state = ConversationState::new(ctx, load_tools().unwrap(), None).await;

        let conversation_start_context = "conversation start context";
        let prompt_context = "prompt context";

        // Simulate conversation flow
        conversation_state.append_new_user_message("start".to_string()).await;
        for i in 0..=(MAX_CONVERSATION_STATE_HISTORY_LEN + 100) {
            let s = conversation_state
                .as_sendable_conversation_state(Some(ExtraContext {
                    general_context: Some(conversation_start_context.to_string()),
                    user_input_context: Some(prompt_context.to_string()),
                }))
                .await;
            let hist = s.history.as_ref().unwrap();
            #[allow(clippy::match_wildcard_for_single_variants)]
            match &hist[0] {
                ChatMessage::UserInputMessage(user) => {
                    assert!(
                        user.content.contains(conversation_start_context),
                        "expected to contain '{conversation_start_context}', instead found: {}",
                        user.content
                    );
                },
                _ => panic!("Expected user message."),
            }
            assert!(
                s.user_input_message.content.contains(prompt_context),
                "expected to contain '{prompt_context}', instead found: {}",
                s.user_input_message.content
            );

            conversation_state.push_assistant_message(AssistantResponseMessage {
                message_id: None,
                content: i.to_string(),
                tool_uses: None,
            });
            conversation_state.append_new_user_message(i.to_string()).await;
        }
    }
}
