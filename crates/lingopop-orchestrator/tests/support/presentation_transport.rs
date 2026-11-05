//! Test scheduling barrier: a provider response is released only after Loading was acked.
use lingopop_core::{
    errors::TranslationError,
    http::{HttpRequest, HttpResponse, Transport},
};
use std::sync::Arc;

pub struct AfterPresentation {
    pub transport: Arc<dyn Transport>,
    pub flush: Arc<dyn Fn() + Send + Sync>,
}
#[async_trait::async_trait]
impl Transport for AfterPresentation {
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse, TranslationError> {
        (self.flush)();
        self.transport.send(request).await
    }
}
