#!/bin/bash
# Comprehensive End-to-End Test for AWS Bedrock Integration

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Test configuration
ROUTIIUM_HOST="http://localhost:8080"
TEST_API_KEY="sk-test123"

echo -e "${YELLOW}=========================================${NC}"
echo -e "${YELLOW}AWS Bedrock E2E Test Suite${NC}"
echo -e "${YELLOW}=========================================${NC}\n"

# Function to check if Routiium is running
check_server() {
    echo -e "${YELLOW}[1/9] Checking if Routiium server is running...${NC}"
    if curl -s "${ROUTIIUM_HOST}/health" > /dev/null 2>&1; then
        echo -e "${GREEN}✓ Server is running${NC}\n"
        return 0
    else
        echo -e "${RED}✗ Server is not running. Please start Routiium first.${NC}"
        echo -e "  Run: cd routiium && cargo run --release\n"
        return 1
    fi
}

# Function to test basic text generation
test_basic_text() {
    echo -e "${YELLOW}[2/9] Testing basic text generation with Claude 3.5 Sonnet...${NC}"

    response=$(curl -s -w "\nHTTP_STATUS:%{http_code}" "${ROUTIIUM_HOST}/v1/chat/completions" \
        -H "Content-Type: application/json" \
        -H "Authorization: Bearer ${TEST_API_KEY}" \
        -d '{
            "model": "anthropic.claude-3-5-sonnet-20241022-v2:0",
            "messages": [
                {
                    "role": "user",
                    "content": "Say hello in exactly 5 words."
                }
            ],
            "max_tokens": 50
        }')

    http_code=$(echo "$response" | grep "HTTP_STATUS" | cut -d: -f2)
    body=$(echo "$response" | sed '/HTTP_STATUS/d')

    if [ "$http_code" = "200" ]; then
        echo -e "${GREEN}✓ Request successful${NC}"
        echo "Response preview:"
        echo "$body" | jq -r '.choices[0].message.content' | head -c 100
        echo -e "\n"

        # Verify response structure
        if echo "$body" | jq -e '.id and .choices and .usage' > /dev/null 2>&1; then
            echo -e "${GREEN}✓ Response has correct Chat Completions format${NC}\n"
        else
            echo -e "${RED}✗ Response missing required fields${NC}\n"
            return 1
        fi
    else
        echo -e "${RED}✗ Request failed with status: $http_code${NC}"
        echo "$body" | jq '.' 2>/dev/null || echo "$body"
        echo ""
        return 1
    fi
}

# Function to test multimodal (image) input
test_multimodal() {
    echo -e "${YELLOW}[3/9] Testing multimodal input (image + text)...${NC}"

    # Create a simple test image (1x1 red pixel PNG in base64)
    test_image="iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8DwHwAFBQIAX8jx0gAAAABJRU5ErkJggg=="

    response=$(curl -s -w "\nHTTP_STATUS:%{http_code}" "${ROUTIIUM_HOST}/v1/chat/completions" \
        -H "Content-Type: application/json" \
        -H "Authorization: Bearer ${TEST_API_KEY}" \
        -d "{
            \"model\": \"anthropic.claude-3-5-sonnet-20241022-v2:0\",
            \"messages\": [
                {
                    \"role\": \"user\",
                    \"content\": [
                        {
                            \"type\": \"text\",
                            \"text\": \"What color is this image? Respond in one word.\"
                        },
                        {
                            \"type\": \"image_url\",
                            \"image_url\": {
                                \"url\": \"data:image/png;base64,${test_image}\"
                            }
                        }
                    ]
                }
            ],
            \"max_tokens\": 50
        }")

    http_code=$(echo "$response" | grep "HTTP_STATUS" | cut -d: -f2)
    body=$(echo "$response" | sed '/HTTP_STATUS/d')

    if [ "$http_code" = "200" ]; then
        echo -e "${GREEN}✓ Multimodal request successful${NC}"
        echo "Response:"
        echo "$body" | jq -r '.choices[0].message.content'
        echo ""
    else
        echo -e "${RED}✗ Multimodal request failed with status: $http_code${NC}"
        echo "$body" | jq '.' 2>/dev/null || echo "$body"
        echo ""
        return 1
    fi
}

# Function to test tool calling
test_tool_calling() {
    echo -e "${YELLOW}[4/9] Testing tool calling (function definitions)...${NC}"

    response=$(curl -s -w "\nHTTP_STATUS:%{http_code}" "${ROUTIIUM_HOST}/v1/chat/completions" \
        -H "Content-Type: application/json" \
        -H "Authorization: Bearer ${TEST_API_KEY}" \
        -d '{
            "model": "anthropic.claude-3-5-sonnet-20241022-v2:0",
            "messages": [
                {
                    "role": "user",
                    "content": "What is the weather in San Francisco?"
                }
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
                                    "description": "The city and state, e.g. San Francisco, CA"
                                },
                                "unit": {
                                    "type": "string",
                                    "enum": ["celsius", "fahrenheit"],
                                    "description": "The temperature unit"
                                }
                            },
                            "required": ["location"]
                        }
                    }
                }
            ],
            "max_tokens": 200
        }')

    http_code=$(echo "$response" | grep "HTTP_STATUS" | cut -d: -f2)
    body=$(echo "$response" | sed '/HTTP_STATUS/d')

    if [ "$http_code" = "200" ]; then
        echo -e "${GREEN}✓ Tool calling request successful${NC}"

        # Check if tool was called
        if echo "$body" | jq -e '.choices[0].message.tool_calls' > /dev/null 2>&1; then
            echo -e "${GREEN}✓ Tool calls present in response${NC}"
            echo "Tool called:"
            echo "$body" | jq '.choices[0].message.tool_calls[0].function.name'
            echo "Arguments:"
            echo "$body" | jq '.choices[0].message.tool_calls[0].function.arguments'
            echo ""
        else
            echo -e "${YELLOW}! Model responded with text instead of tool call${NC}"
            echo "Response:"
            echo "$body" | jq -r '.choices[0].message.content' | head -c 200
            echo -e "\n"
        fi
    else
        echo -e "${RED}✗ Tool calling request failed with status: $http_code${NC}"
        echo "$body" | jq '.' 2>/dev/null || echo "$body"
        echo ""
        return 1
    fi
}

# Function to test token usage tracking
test_usage_tracking() {
    echo -e "${YELLOW}[5/9] Testing token usage tracking...${NC}"

    response=$(curl -s "${ROUTIIUM_HOST}/v1/chat/completions" \
        -H "Content-Type: application/json" \
        -H "Authorization: Bearer ${TEST_API_KEY}" \
        -d '{
            "model": "anthropic.claude-3-5-sonnet-20241022-v2:0",
            "messages": [
                {
                    "role": "user",
                    "content": "Count to 5."
                }
            ],
            "max_tokens": 50
        }')

    if echo "$response" | jq -e '.usage.prompt_tokens and .usage.completion_tokens and .usage.total_tokens' > /dev/null 2>&1; then
        echo -e "${GREEN}✓ Usage tracking working${NC}"
        echo "Token usage:"
        echo "$response" | jq '.usage'
        echo ""
    else
        echo -e "${RED}✗ Usage tracking not working properly${NC}"
        echo "$response" | jq '.'
        echo ""
        return 1
    fi
}

# Function to test different Bedrock models
test_multiple_models() {
    echo -e "${YELLOW}[6/9] Testing multiple Bedrock model families...${NC}"

    # Test with Claude 3 Haiku (faster, cheaper model)
    echo "  Testing Claude 3 Haiku..."
    response=$(curl -s -w "\nHTTP_STATUS:%{http_code}" "${ROUTIIUM_HOST}/v1/chat/completions" \
        -H "Content-Type: application/json" \
        -H "Authorization: Bearer ${TEST_API_KEY}" \
        -d '{
            "model": "anthropic.claude-3-haiku-20240307-v1:0",
            "messages": [
                {
                    "role": "user",
                    "content": "Say hi"
                }
            ],
            "max_tokens": 20
        }')

    http_code=$(echo "$response" | grep "HTTP_STATUS" | cut -d: -f2)

    if [ "$http_code" = "200" ]; then
        echo -e "  ${GREEN}✓ Claude 3 Haiku working${NC}"
    else
        echo -e "  ${YELLOW}! Claude 3 Haiku failed (status: $http_code)${NC}"
    fi

    echo ""
}

# Function to test Mistral models
test_mistral() {
    echo -e "${YELLOW}[7/9] Testing Mistral model...${NC}"

    response=$(curl -s -w "\nHTTP_STATUS:%{http_code}" "${ROUTIIUM_HOST}/v1/chat/completions" \
        -H "Content-Type: application/json" \
        -H "Authorization: Bearer ${TEST_API_KEY}" \
        -d '{
            "model": "mistral.mistral-7b-instruct-v0:2",
            "messages": [
                {
                    "role": "user",
                    "content": "Say hello in exactly 3 words."
                }
            ],
            "max_tokens": 50
        }')

    http_code=$(echo "$response" | grep "HTTP_STATUS" | cut -d: -f2)
    body=$(echo "$response" | sed '/HTTP_STATUS/d')

    if [ "$http_code" = "200" ]; then
        echo -e "${GREEN}✓ Mistral request successful${NC}"
        echo "Response preview:"
        echo "$body" | jq -r '.choices[0].message.content' | head -c 100
        echo -e "\n"

        # Verify response structure
        if echo "$body" | jq -e '.id and .choices and .usage' > /dev/null 2>&1; then
            echo -e "${GREEN}✓ Response has correct format${NC}\n"
        else
            echo -e "${RED}✗ Response missing required fields${NC}\n"
            return 1
        fi
    else
        echo -e "${YELLOW}! Mistral request failed (model may not be available in your region)${NC}"
        echo -e "  Status: $http_code"
        echo "$body" | jq '.' 2>/dev/null || echo "$body"
        echo ""
        # Don't fail the test if Mistral is not available
        return 0
    fi
}

# Function to test error handling
test_error_handling() {
    echo -e "${YELLOW}[8/9] Testing error handling...${NC}"

    # Test with invalid model
    echo "  Testing invalid model ID..."
    response=$(curl -s -w "\nHTTP_STATUS:%{http_code}" "${ROUTIIUM_HOST}/v1/chat/completions" \
        -H "Content-Type: application/json" \
        -H "Authorization: Bearer ${TEST_API_KEY}" \
        -d '{
            "model": "invalid.model.id",
            "messages": [
                {
                    "role": "user",
                    "content": "test"
                }
            ],
            "max_tokens": 10
        }')

    http_code=$(echo "$response" | grep "HTTP_STATUS" | cut -d: -f2)

    if [ "$http_code" != "200" ]; then
        echo -e "  ${GREEN}✓ Error handling working (status: $http_code)${NC}"
    else
        echo -e "  ${YELLOW}! Invalid model request succeeded unexpectedly${NC}"
    fi

    echo ""
}

# Function to test system messages
test_system_message() {
    echo -e "${YELLOW}[9/9] Testing system message handling...${NC}"

    response=$(curl -s -w "\nHTTP_STATUS:%{http_code}" "${ROUTIIUM_HOST}/v1/chat/completions" \
        -H "Content-Type: application/json" \
        -H "Authorization: Bearer ${TEST_API_KEY}" \
        -d '{
            "model": "anthropic.claude-3-5-sonnet-20241022-v2:0",
            "messages": [
                {
                    "role": "system",
                    "content": "You are a helpful assistant who only speaks in haikus."
                },
                {
                    "role": "user",
                    "content": "Tell me about the ocean."
                }
            ],
            "max_tokens": 100
        }')

    http_code=$(echo "$response" | grep "HTTP_STATUS" | cut -d: -f2)
    body=$(echo "$response" | sed '/HTTP_STATUS/d')

    if [ "$http_code" = "200" ]; then
        echo -e "${GREEN}✓ System message request successful${NC}"
        echo "Response:"
        echo "$body" | jq -r '.choices[0].message.content'
        echo ""
    else
        echo -e "${RED}✗ System message request failed with status: $http_code${NC}"
        echo "$body" | jq '.' 2>/dev/null || echo "$body"
        echo ""
        return 1
    fi
}

# Run all tests
main() {
    passed=0
    failed=0

    if ! check_server; then
        exit 1
    fi

    for test_func in test_basic_text test_multimodal test_tool_calling test_usage_tracking test_multiple_models test_mistral test_error_handling test_system_message; do
        if $test_func; then
            ((passed++))
        else
            ((failed++))
        fi
        sleep 1  # Brief pause between tests
    done

    echo -e "${YELLOW}=====================================${NC}"
    echo -e "${YELLOW}Test Summary${NC}"
    echo -e "${YELLOW}=====================================${NC}"
    echo -e "${GREEN}Passed: $passed${NC}"
    echo -e "${RED}Failed: $failed${NC}"
    echo ""

    if [ $failed -eq 0 ]; then
        echo -e "${GREEN}✓ All tests passed!${NC}\n"
        exit 0
    else
        echo -e "${RED}✗ Some tests failed${NC}\n"
        exit 1
    fi
}

main "$@"
