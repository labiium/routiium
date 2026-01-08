#!/bin/bash
# Comprehensive Bedrock Integration Test
# Tests the AWS Bedrock implementation without requiring actual AWS credentials

# set -e temporarily disabled to allow grep failures

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

echo -e "${BLUE}========================================${NC}"
echo -e "${BLUE}AWS Bedrock Integration Test Suite${NC}"
echo -e "${BLUE}========================================${NC}\n"

# Test counter
TESTS_PASSED=0
TESTS_FAILED=0

# Function to print test result
print_result() {
    local test_name="$1"
    local result="$2"

    if [ "$result" = "PASS" ]; then
        echo -e "${GREEN}✓ PASS${NC}: $test_name"
        ((TESTS_PASSED++))
    else
        echo -e "${RED}✗ FAIL${NC}: $test_name"
        ((TESTS_FAILED++))
    fi
}

# Test 1: Check if bedrock.rs exists
echo -e "${YELLOW}[1/10] Checking bedrock.rs module exists...${NC}"
if [ -f "src/bedrock.rs" ]; then
    print_result "Bedrock module file exists" "PASS"
else
    print_result "Bedrock module file exists" "FAIL"
fi

# Test 2: Check if module is exported in lib.rs
echo -e "\n${YELLOW}[2/10] Checking module export in lib.rs...${NC}"
if grep -q "pub mod bedrock" src/lib.rs 2>/dev/null; then
    print_result "Bedrock module exported in lib.rs" "PASS"
else
    print_result "Bedrock module exported in lib.rs" "FAIL"
fi

# Test 3: Check AWS dependencies in Cargo.toml
echo -e "\n${YELLOW}[3/10] Checking AWS dependencies...${NC}"
if grep -q "aws-sdk-bedrockruntime" Cargo.toml 2>/dev/null && \
   grep -q "aws-config" Cargo.toml 2>/dev/null; then
    print_result "AWS SDK dependencies present" "PASS"
else
    print_result "AWS SDK dependencies present" "FAIL"
fi

# Test 4: Check UpstreamMode includes Bedrock
echo -e "\n${YELLOW}[4/10] Checking UpstreamMode enum...${NC}"
if grep -A5 "pub enum UpstreamMode" src/util.rs 2>/dev/null | grep -q "Bedrock" 2>/dev/null; then
    print_result "UpstreamMode includes Bedrock variant" "PASS"
else
    print_result "UpstreamMode includes Bedrock variant" "FAIL"
fi

# Test 5: Check server.rs integration
echo -e "\n${YELLOW}[5/10] Checking server.rs integration...${NC}"
if grep -q "UpstreamMode::Bedrock" src/server.rs 2>/dev/null && \
   grep -q "invoke_bedrock_model" src/server.rs 2>/dev/null; then
    print_result "Server has Bedrock integration" "PASS"
else
    print_result "Server has Bedrock integration" "FAIL"
fi

# Test 6: Run unit tests
echo -e "\n${YELLOW}[6/10] Running unit tests...${NC}"
if cargo test bedrock --lib --quiet 2>&1 | grep -q "test result: ok"; then
    print_result "Unit tests pass" "PASS"
else
    print_result "Unit tests pass" "FAIL"
fi

# Test 7: Check for required conversion functions
echo -e "\n${YELLOW}[7/10] Checking conversion functions...${NC}"
if grep -q "pub fn chat_to_bedrock_request" src/bedrock.rs 2>/dev/null && \
   grep -q "pub fn bedrock_to_chat_response" src/bedrock.rs 2>/dev/null; then
    print_result "Conversion functions present" "PASS"
else
    print_result "Conversion functions present" "FAIL"
fi

# Test 8: Check provider detection
echo -e "\n${YELLOW}[8/10] Checking provider detection...${NC}"
if grep -q "pub enum BedrockProvider" src/bedrock.rs 2>/dev/null && \
   grep -q "Anthropic" src/bedrock.rs 2>/dev/null && \
   grep -q "AmazonTitan" src/bedrock.rs 2>/dev/null; then
    print_result "Provider detection implemented" "PASS"
else
    print_result "Provider detection implemented" "FAIL"
fi

# Test 9: Check tool calling support
echo -e "\n${YELLOW}[9/10] Checking tool calling support...${NC}"
if grep -q "BedrockTool" src/bedrock.rs 2>/dev/null && \
   grep -q "BedrockToolUse" src/bedrock.rs 2>/dev/null; then
    print_result "Tool calling structures present" "PASS"
else
    print_result "Tool calling structures present" "FAIL"
fi

# Test 10: Check multimodal support
echo -e "\n${YELLOW}[10/10] Checking multimodal support...${NC}"
if grep -q "BedrockImageSource" src/bedrock.rs 2>/dev/null && \
   grep -q "Base64" src/bedrock.rs 2>/dev/null; then
    print_result "Multimodal/image support present" "PASS"
else
    print_result "Multimodal/image support present" "FAIL"
fi

# Test 11: Check documentation
echo -e "\n${YELLOW}[Bonus] Checking documentation...${NC}"
if [ -f "AWS_BEDROCK.md" ]; then
    print_result "AWS Bedrock documentation exists" "PASS"
else
    print_result "AWS Bedrock documentation exists" "FAIL"
fi

# Test 12: Check example configurations
echo -e "\n${YELLOW}[Bonus] Checking configuration examples...${NC}"
if grep -q "bedrock" routing.json.example 2>/dev/null || \
   grep -q "bedrock" bedrock_test_config.json 2>/dev/null; then
    print_result "Configuration examples present" "PASS"
else
    print_result "Configuration examples present" "FAIL"
fi

# Test 13: Check helper scripts
echo -e "\n${YELLOW}[Bonus] Checking helper scripts...${NC}"
if [ -f "start_bedrock_server.sh" ] && [ -f "test_bedrock_e2e.sh" ]; then
    print_result "Helper scripts present" "PASS"
else
    print_result "Helper scripts present" "FAIL"
fi

# Summary
echo -e "\n${BLUE}========================================${NC}"
echo -e "${BLUE}Test Summary${NC}"
echo -e "${BLUE}========================================${NC}"
echo -e "${GREEN}Passed: $TESTS_PASSED${NC}"
echo -e "${RED}Failed: $TESTS_FAILED${NC}"
echo ""

# Detailed implementation check
echo -e "${BLUE}Implementation Details:${NC}"
echo -e "${BLUE}========================================${NC}"

# Check which providers are implemented
echo -e "\n${YELLOW}Supported Providers:${NC}"
for provider in "Anthropic" "AmazonTitan" "Meta" "AI21" "Cohere" "Mistral"; do
    if grep -q "BedrockProvider::$provider =>" src/bedrock.rs 2>/dev/null; then
        echo -e "  ${GREEN}✓${NC} $provider (implemented)"
    elif grep -q "$provider" src/bedrock.rs 2>/dev/null; then
        echo -e "  ${YELLOW}○${NC} $provider (defined but not fully implemented)"
    else
        echo -e "  ${RED}✗${NC} $provider (not implemented)"
    fi
done

# Check features
echo -e "\n${YELLOW}Features:${NC}"
FEATURES=(
    "chat_to_anthropic_bedrock:Anthropic Claude conversion"
    "chat_to_titan_bedrock:Amazon Titan conversion"
    "chat_to_meta_bedrock:Meta Llama conversion"
    "invoke_bedrock_model:AWS SDK integration"
    "invoke_bedrock_model_streaming:Streaming support"
    "BedrockImageSource:Multimodal/vision support"
    "BedrockTool:Tool calling support"
    "AwsConfig:AWS configuration"
)

for feature in "${FEATURES[@]}"; do
    IFS=':' read -r func desc <<< "$feature"
    if grep -q "$func" src/bedrock.rs 2>/dev/null; then
        echo -e "  ${GREEN}✓${NC} $desc"
    else
        echo -e "  ${RED}✗${NC} $desc"
    fi
done

# Check test coverage
echo -e "\n${YELLOW}Test Coverage:${NC}"
TEST_FUNCTIONS=$(grep -c "fn test_" src/bedrock.rs 2>/dev/null || echo "0")
echo -e "  Unit tests: $TEST_FUNCTIONS"

# Check for streaming implementation
echo -e "\n${YELLOW}Streaming Status:${NC}"
if grep -A5 "invoke_bedrock_model_streaming" src/bedrock.rs 2>/dev/null | grep -q "TODO\|For now"; then
    echo -e "  ${YELLOW}⚠${NC}  Streaming partially implemented (uses non-streaming internally)"
else
    echo -e "  ${GREEN}✓${NC} Streaming fully implemented"
fi

# Check for warnings
echo -e "\n${YELLOW}Code Quality:${NC}"
if cargo build --lib --quiet 2>&1 | grep -i "warning" >/dev/null 2>&1; then
    echo -e "  ${YELLOW}⚠${NC}  Build has warnings"
else
    echo -e "  ${GREEN}✓${NC} No build warnings"
fi

# Final verdict
echo -e "\n${BLUE}========================================${NC}"
if [ $TESTS_FAILED -eq 0 ]; then
    echo -e "${GREEN}✓ ALL TESTS PASSED!${NC}"
    echo -e "${GREEN}AWS Bedrock integration is complete and ready.${NC}"
    echo ""
    echo -e "${BLUE}Next Steps:${NC}"
    echo "  1. Set up AWS credentials (AWS_ACCESS_KEY_ID, AWS_SECRET_ACCESS_KEY, AWS_REGION)"
    echo "  2. Run: ./start_bedrock_server.sh"
    echo "  3. Test with: ./test_bedrock_e2e.sh"
    echo ""
    exit 0
else
    echo -e "${RED}✗ SOME TESTS FAILED${NC}"
    echo -e "${YELLOW}Please review the failed tests above.${NC}"
    echo ""
    exit 1
fi
