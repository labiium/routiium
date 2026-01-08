#!/bin/bash
# Start Routiium server with AWS Bedrock configuration

set -e

# Load AWS credentials from .env file
if [ -f .env ]; then
    export $(grep -v '^#' .env | grep -E '^AWS_' | xargs)
    echo "✓ Loaded AWS credentials from .env"
else
    echo "⚠ Warning: .env file not found"
    echo "  Make sure AWS_REGION, AWS_ACCESS_KEY_ID, and AWS_SECRET_ACCESS_KEY are set"
fi

# Verify AWS credentials are set
if [ -z "$AWS_REGION" ] || [ -z "$AWS_ACCESS_KEY_ID" ] || [ -z "$AWS_SECRET_ACCESS_KEY" ]; then
    echo "✗ Error: AWS credentials not properly configured"
    echo "  Required environment variables:"
    echo "    - AWS_REGION"
    echo "    - AWS_ACCESS_KEY_ID"
    echo "    - AWS_SECRET_ACCESS_KEY"
    exit 1
fi

echo "AWS Configuration:"
echo "  Region: $AWS_REGION"
echo "  Access Key ID: ${AWS_ACCESS_KEY_ID:0:10}..."
echo ""

# Generate test API key
echo "Generating test API key..."
python3 scripts/generate_api_key.py --key sk-test123 --output tmp/test_api_keys.json

# Set environment variables for Bedrock routing
export ROUTIIUM_BACKENDS="bedrock:https://bedrock-runtime.$AWS_REGION.amazonaws.com:anthropic.*:bedrock"
export ROUTIIUM_API_KEYS_FILE="tmp/test_api_keys.json"
export ROUTIIUM_PORT="8080"

echo "Starting Routiium with Bedrock support..."
echo "  Backend: https://bedrock-runtime.$AWS_REGION.amazonaws.com"
echo "  Port: $ROUTIIUM_PORT"
echo ""

# Build and run
cargo run --release

