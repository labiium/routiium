#!/bin/bash
# Example script to test AWS Bedrock integration with Routiium

# Colors for output
GREEN='\033[0;32m'
BLUE='\033[0;34m'
RED='\033[0;31m'
NC='\033[0m' # No Color

echo -e "${BLUE}AWS Bedrock Integration Test${NC}\n"

# Check if Routiium is running
if ! curl -s http://localhost:8088/status > /dev/null 2>&1; then
    echo -e "${RED}Error: Routiium is not running on localhost:8088${NC}"
    echo "Please start Routiium first:"
    echo "  cargo run --release"
    exit 1
fi

# Check for AWS credentials
if [ -z "$AWS_ACCESS_KEY_ID" ] || [ -z "$AWS_SECRET_ACCESS_KEY" ]; then
    echo -e "${RED}Error: AWS credentials not found${NC}"
    echo "Please set AWS environment variables:"
    echo "  export AWS_ACCESS_KEY_ID=your-access-key"
    echo "  export AWS_SECRET_ACCESS_KEY=your-secret-key"
    echo "  export AWS_REGION=us-east-1"
    exit 1
fi

# Get or generate API key
API_KEY="${ROUTIIUM_API_KEY:-}"
if [ -z "$API_KEY" ]; then
    echo -e "${BLUE}Generating API key...${NC}"
    KEY_RESPONSE=$(curl -s -X POST http://localhost:8088/keys/generate \
        -H "Content-Type: application/json" \
        -d '{"label": "bedrock-test", "ttl_seconds": 3600}')
    
    API_KEY=$(echo "$KEY_RESPONSE" | grep -o '"bearer":"[^"]*"' | cut -d'"' -f4)
    
    if [ -z "$API_KEY" ]; then
        echo -e "${RED}Failed to generate API key${NC}"
        exit 1
    fi
    echo -e "${GREEN}API key generated: ${API_KEY:0:20}...${NC}\n"
fi

# Test 1: Basic Chat Request
echo -e "${BLUE}Test 1: Basic Chat Request${NC}"
curl -s -X POST http://localhost:8088/v1/chat/completions \
  -H "Authorization: Bearer $API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "anthropic.claude-3-haiku-20240307-v1:0",
    "messages": [
      {"role": "user", "content": "Say hello in one sentence."}
    ],
    "max_tokens": 100
  }' | jq -r '.choices[0].message.content' 2>/dev/null || echo -e "${RED}Request failed${NC}"

echo -e "\n"

# Test 2: Tool Calling
echo -e "${BLUE}Test 2: Tool/Function Calling${NC}"
TOOL_RESPONSE=$(curl -s -X POST http://localhost:8088/v1/chat/completions \
  -H "Authorization: Bearer $API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "anthropic.claude-3-sonnet-20240229-v1:0",
    "messages": [
      {"role": "user", "content": "What is the weather in San Francisco?"}
    ],
    "tools": [
      {
        "type": "function",
        "function": {
          "name": "get_weather",
          "description": "Get the current weather for a location",
          "parameters": {
            "type": "object",
            "properties": {
              "location": {
                "type": "string",
                "description": "City and state"
              }
            },
            "required": ["location"]
          }
        }
      }
    ],
    "tool_choice": "auto",
    "max_tokens": 500
  }')

echo "$TOOL_RESPONSE" | jq -r '.choices[0].message.tool_calls[0].function.name // "No tool call"' 2>/dev/null || echo -e "${RED}Request failed${NC}"

echo -e "\n"

# Test 3: Streaming
echo -e "${BLUE}Test 3: Streaming Response${NC}"
curl -s -N -X POST http://localhost:8088/v1/chat/completions \
  -H "Authorization: Bearer $API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "anthropic.claude-3-haiku-20240307-v1:0",
    "messages": [
      {"role": "user", "content": "Count from 1 to 5."}
    ],
    "stream": true,
    "max_tokens": 100
  }' 2>/dev/null | head -n 10

echo -e "\n"

# Test 4: Using Router Alias (if configured)
echo -e "${BLUE}Test 4: Using Router Alias${NC}"
curl -s -X POST http://localhost:8088/v1/chat/completions \
  -H "Authorization: Bearer $API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "bedrock-claude-haiku",
    "messages": [
      {"role": "user", "content": "Say goodbye in one sentence."}
    ],
    "max_tokens": 100
  }' | jq -r '.choices[0].message.content // "Alias not configured"' 2>/dev/null

echo -e "\n${GREEN}Tests completed!${NC}"
echo -e "\nFor multimodal (vision) testing, see AWS_BEDROCK.md for examples with images."

