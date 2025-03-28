// Code generated by software.amazon.smithy.rust.codegen.smithy-rs. DO NOT EDIT.
#[allow(missing_docs)] // documentation missing in model
#[non_exhaustive]
#[derive(::std::clone::Clone, ::std::cmp::PartialEq, ::std::fmt::Debug)]
pub struct SendEventOutput {
    #[allow(missing_docs)] // documentation missing in model
    pub client_token: ::std::option::Option<::std::string::String>,
    /// Currently supported providers for receiving events.
    pub provider_id: ::std::option::Option<crate::types::SupportedProviderId>,
    #[allow(missing_docs)] // documentation missing in model
    pub event_id: ::std::option::Option<::std::string::String>,
    #[allow(missing_docs)] // documentation missing in model
    pub event_version: ::std::option::Option<::std::string::String>,
    _request_id: Option<String>,
}
impl SendEventOutput {
    #[allow(missing_docs)] // documentation missing in model
    pub fn client_token(&self) -> ::std::option::Option<&str> {
        self.client_token.as_deref()
    }

    /// Currently supported providers for receiving events.
    pub fn provider_id(&self) -> ::std::option::Option<&crate::types::SupportedProviderId> {
        self.provider_id.as_ref()
    }

    #[allow(missing_docs)] // documentation missing in model
    pub fn event_id(&self) -> ::std::option::Option<&str> {
        self.event_id.as_deref()
    }

    #[allow(missing_docs)] // documentation missing in model
    pub fn event_version(&self) -> ::std::option::Option<&str> {
        self.event_version.as_deref()
    }
}
impl ::aws_types::request_id::RequestId for SendEventOutput {
    fn request_id(&self) -> Option<&str> {
        self._request_id.as_deref()
    }
}
impl SendEventOutput {
    /// Creates a new builder-style object to manufacture
    /// [`SendEventOutput`](crate::operation::send_event::SendEventOutput).
    pub fn builder() -> crate::operation::send_event::builders::SendEventOutputBuilder {
        crate::operation::send_event::builders::SendEventOutputBuilder::default()
    }
}

/// A builder for [`SendEventOutput`](crate::operation::send_event::SendEventOutput).
#[derive(::std::clone::Clone, ::std::cmp::PartialEq, ::std::default::Default, ::std::fmt::Debug)]
#[non_exhaustive]
pub struct SendEventOutputBuilder {
    pub(crate) client_token: ::std::option::Option<::std::string::String>,
    pub(crate) provider_id: ::std::option::Option<crate::types::SupportedProviderId>,
    pub(crate) event_id: ::std::option::Option<::std::string::String>,
    pub(crate) event_version: ::std::option::Option<::std::string::String>,
    _request_id: Option<String>,
}
impl SendEventOutputBuilder {
    #[allow(missing_docs)] // documentation missing in model
    pub fn client_token(mut self, input: impl ::std::convert::Into<::std::string::String>) -> Self {
        self.client_token = ::std::option::Option::Some(input.into());
        self
    }

    #[allow(missing_docs)] // documentation missing in model
    pub fn set_client_token(mut self, input: ::std::option::Option<::std::string::String>) -> Self {
        self.client_token = input;
        self
    }

    #[allow(missing_docs)] // documentation missing in model
    pub fn get_client_token(&self) -> &::std::option::Option<::std::string::String> {
        &self.client_token
    }

    /// Currently supported providers for receiving events.
    pub fn provider_id(mut self, input: crate::types::SupportedProviderId) -> Self {
        self.provider_id = ::std::option::Option::Some(input);
        self
    }

    /// Currently supported providers for receiving events.
    pub fn set_provider_id(mut self, input: ::std::option::Option<crate::types::SupportedProviderId>) -> Self {
        self.provider_id = input;
        self
    }

    /// Currently supported providers for receiving events.
    pub fn get_provider_id(&self) -> &::std::option::Option<crate::types::SupportedProviderId> {
        &self.provider_id
    }

    #[allow(missing_docs)] // documentation missing in model
    pub fn event_id(mut self, input: impl ::std::convert::Into<::std::string::String>) -> Self {
        self.event_id = ::std::option::Option::Some(input.into());
        self
    }

    #[allow(missing_docs)] // documentation missing in model
    pub fn set_event_id(mut self, input: ::std::option::Option<::std::string::String>) -> Self {
        self.event_id = input;
        self
    }

    #[allow(missing_docs)] // documentation missing in model
    pub fn get_event_id(&self) -> &::std::option::Option<::std::string::String> {
        &self.event_id
    }

    #[allow(missing_docs)] // documentation missing in model
    pub fn event_version(mut self, input: impl ::std::convert::Into<::std::string::String>) -> Self {
        self.event_version = ::std::option::Option::Some(input.into());
        self
    }

    #[allow(missing_docs)] // documentation missing in model
    pub fn set_event_version(mut self, input: ::std::option::Option<::std::string::String>) -> Self {
        self.event_version = input;
        self
    }

    #[allow(missing_docs)] // documentation missing in model
    pub fn get_event_version(&self) -> &::std::option::Option<::std::string::String> {
        &self.event_version
    }

    pub(crate) fn _request_id(mut self, request_id: impl Into<String>) -> Self {
        self._request_id = Some(request_id.into());
        self
    }

    pub(crate) fn _set_request_id(&mut self, request_id: Option<String>) -> &mut Self {
        self._request_id = request_id;
        self
    }

    /// Consumes the builder and constructs a
    /// [`SendEventOutput`](crate::operation::send_event::SendEventOutput).
    pub fn build(self) -> crate::operation::send_event::SendEventOutput {
        crate::operation::send_event::SendEventOutput {
            client_token: self.client_token,
            provider_id: self.provider_id,
            event_id: self.event_id,
            event_version: self.event_version,
            _request_id: self._request_id,
        }
    }
}
