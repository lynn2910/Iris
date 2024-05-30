use std::sync::Arc;
use reqwest::{Error, Request, RequestBuilder, Response};
use reqwest::header::AUTHORIZATION;

/// This structure contains the HTTP client
pub(crate) struct RestClient {
    pub(crate) client: reqwest::Client,
    token: Arc<String>
}

impl RestClient {
    pub(crate) fn new(token: Arc<String>) -> Self {
        let client = reqwest::Client::new();
        Self { client, token: token.clone() }
    }

    pub(crate) async fn send_request(&self, request: HttpRequestBuilder) -> Result<Response, Error> {
        let req = request.build(&self.token)?;
        self.client.execute(req).await
    }
    
    pub(crate) fn get_token(&self) -> Arc<String> {
        self.token.clone()
    }
}

pub(crate) struct HttpRequestBuilder {
    pub request_builder: RequestBuilder,
    require_token: bool
}

impl HttpRequestBuilder {
    pub fn new(request_builder: RequestBuilder) -> Self {
        Self { request_builder, require_token: true }
    }
    
    pub fn require_token(mut self, require_token: bool) -> Self {
        self.require_token = require_token;
        self
    }
    
    pub fn build(self, token: &Arc<String>) -> reqwest::Result<Request> {
        if self.require_token {
            return self.request_builder
                .header(AUTHORIZATION, format!("Bot {token}"))
                .build()
        }
        
        self.request_builder.build()
    }
}


/// Helper to build urls
#[macro_export]
macro_rules! url {
    ($($arg:tt)*) => {{
        let mut formatted = format!($($arg)*);
        if formatted.starts_with('/') { formatted = format!("{}{formatted}", $crate::API_URL); }
        else  { formatted = format!("{}/{formatted}", $crate::API_URL); }

        formatted
    }};
}
