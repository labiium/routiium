#!/bin/bash
# Start Routiium server with AWS Bedrock configuration

set -e

# Load AWS credentials from .env file
if [ -f .env ]; then
    export $(grep -v '^#' .env | grep -E '^AWS_' | xargs)
    echo "✓ Loaded AWS credentials from .env"
else
    echo "⚠ Warning: .env file not found"
    echo "  Make sure AWS_REGION and either AWS credentials or AWS_BEARER_TOKEN_BEDROCK are set"
fi

# Verify the region is set. Bedrock Mantle can authenticate with an API key or the AWS credential chain.
if [ -z "$AWS_REGION" ]; then
    echo "✗ Error: AWS_REGION is not configured"
    echo "  Set AWS_REGION before starting Routiium"
    exit 1
fi

echo "AWS Configuration:"
echo "  Region: $AWS_REGION"
if [ -n "$AWS_BEARER_TOKEN_BEDROCK" ]; then
    echo "  Mantle authentication: Bedrock API key"
else
    echo "  Mantle authentication: AWS credential chain"
fi
echo ""

# Set environment variables for both Bedrock request families.
export ROUTIIUM_BACKENDS="prefix=openai.gpt-5.6-,base=https://bedrock-mantle.$AWS_REGION.api.aws/openai/v1,key_env=AWS_BEARER_TOKEN_BEDROCK,mode=bedrock_mantle;prefix=anthropic.,base=https://bedrock-runtime.$AWS_REGION.amazonaws.com,mode=bedrock"
export ROUTIIUM_MANAGED_MODE="0"
export ROUTIIUM_PORT="8080"

echo "Starting Routiium with Bedrock support..."
echo "  Mantle backend: https://bedrock-mantle.$AWS_REGION.api.aws/openai/v1"
echo "  Legacy backend: https://bedrock-runtime.$AWS_REGION.amazonaws.com"
echo "  Port: $ROUTIIUM_PORT"
echo ""

# Build and run
cargo run --release
