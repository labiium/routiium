# AWS Bedrock Support

Routiium supports AWS Bedrock as an upstream provider in OpenAI-compatible mode.

## What works

- Chat and Responses passthrough via Routiium endpoints.
- Tool-calling translation through the existing conversion pipeline.
- Model routing through aliases and router plans.

## Basic setup

Set the standard AWS credentials used by the AWS SDK:

- `AWS_REGION`
- `AWS_ACCESS_KEY_ID`
- `AWS_SECRET_ACCESS_KEY`
- Optional: `AWS_SESSION_TOKEN`

Then route requests to Bedrock-backed aliases with `--router-config` or `ROUTIIUM_ROUTER_URL`.

## Notes

- Keep Bedrock credentials only on the server side; clients should use Routiium API keys.
- Prefer managed mode and admin token protection for production deployments.
- Validate model IDs and region availability in your AWS account before rollout.
