//! Authentication helpers for the OpenAI-compatible Amazon Bedrock Mantle endpoint.

use anyhow::{Context, Result};
use aws_credential_types::provider::ProvideCredentials;
use aws_sigv4::http_request::{sign, SignableBody, SignableRequest, SigningSettings};
use aws_sigv4::sign::v4;
use aws_smithy_runtime_api::client::identity::Identity;
use std::time::SystemTime;

static AWS_CONFIG: tokio::sync::OnceCell<aws_config::SdkConfig> =
    tokio::sync::OnceCell::const_new();

/// Apply Bedrock API-key or IAM `SigV4` authentication to an HTTP request.
///
/// An explicit bearer token takes precedence. When it is absent, credentials from the
/// standard AWS SDK provider chain are used to sign the request for `bedrock-mantle`.
///
/// # Errors
/// Returns an error when AWS credentials are unavailable or request signing fails.
pub async fn apply_auth(
    builder: reqwest::RequestBuilder,
    url: &str,
    body: &[u8],
    bearer: Option<&str>,
) -> Result<reqwest::RequestBuilder> {
    if let Some(token) = bearer.map(str::trim).filter(|token| !token.is_empty()) {
        return Ok(builder.bearer_auth(token));
    }

    let config = AWS_CONFIG
        .get_or_init(|| async {
            aws_config::defaults(aws_config::BehaviorVersion::latest())
                .load()
                .await
        })
        .await;
    let provider = config
        .credentials_provider()
        .context("AWS credentials are required for Bedrock Mantle SigV4 authentication")?;
    let credentials = provider
        .provide_credentials()
        .await
        .context("failed to load AWS credentials for Bedrock Mantle")?;
    let identity: Identity = credentials.into();

    let headers = [("content-type", "application/json")];
    let signable = SignableRequest::new(
        "POST",
        url,
        headers.iter().copied(),
        SignableBody::Bytes(body),
    )
    .context("failed to build the Bedrock Mantle signable request")?;
    let region = std::env::var("AWS_REGION")
        .or_else(|_| std::env::var("AWS_DEFAULT_REGION"))
        .unwrap_or_else(|_| "us-east-1".to_string());
    let params = v4::SigningParams::builder()
        .identity(&identity)
        .region(&region)
        .name("bedrock-mantle")
        .time(SystemTime::now())
        .settings(SigningSettings::default())
        .build()
        .context("failed to build Bedrock Mantle signing parameters")?
        .into();
    let (instructions, _) = sign(signable, &params)
        .context("failed to sign the Bedrock Mantle request")?
        .into_parts();

    let mut builder = builder;
    for (name, value) in instructions.headers() {
        builder = builder.header(name, value);
    }
    Ok(builder)
}
