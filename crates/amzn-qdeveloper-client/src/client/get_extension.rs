// Code generated by software.amazon.smithy.rust.codegen.smithy-rs. DO NOT EDIT.
impl super::Client {
    /// Constructs a fluent builder for the
    /// [`GetExtension`](crate::operation::get_extension::builders::GetExtensionFluentBuilder)
    /// operation.
    ///
    /// - The fluent builder is configurable:
    ///   - [`extension_id(impl Into<String>)`](crate::operation::get_extension::builders::GetExtensionFluentBuilder::extension_id) / [`set_extension_id(Option<String>)`](crate::operation::get_extension::builders::GetExtensionFluentBuilder::set_extension_id):<br>required: **true**<br>(undocumented)<br>
    /// - On success, responds with
    ///   [`GetExtensionOutput`](crate::operation::get_extension::GetExtensionOutput) with field(s):
    ///   - [`extension_provider(String)`](crate::operation::get_extension::GetExtensionOutput::extension_provider): (undocumented)
    ///   - [`extension_id(String)`](crate::operation::get_extension::GetExtensionOutput::extension_id): (undocumented)
    ///   - [`extension_credential(Option<ExtensionCredential>)`](crate::operation::get_extension::GetExtensionOutput::extension_credential): (undocumented)
    ///   - [`extension_properties(Option<HashMap::<String,
    ///     String>>)`](crate::operation::get_extension::GetExtensionOutput::extension_properties):
    ///     (undocumented)
    ///   - [`creation_time(Option<DateTime>)`](crate::operation::get_extension::GetExtensionOutput::creation_time): (undocumented)
    /// - On failure, responds with
    ///   [`SdkError<GetExtensionError>`](crate::operation::get_extension::GetExtensionError)
    pub fn get_extension(&self) -> crate::operation::get_extension::builders::GetExtensionFluentBuilder {
        crate::operation::get_extension::builders::GetExtensionFluentBuilder::new(self.handle.clone())
    }
}
