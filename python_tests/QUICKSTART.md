# Quick Start Guide - Routiium Python Tests

Get up and running with Python integration tests in under 5 minutes.

## TL;DR - One Command Setup

```bash
cd python_tests
./setup_and_test.sh
```

This single command:
- ✓ Installs `uv` (if needed)
- ✓ Sets up Python environment
- ✓ Builds routiium server
- ✓ Starts server automatically
- ✓ Runs all integration tests
- ✓ Cleans up on exit

## Prerequisites Check

```bash
# Check Rust installation
cargo --version

# Check Python installation
python3 --version

# Verify .env file exists
cat ../.env
```

## Required Environment Variables

Ensure `../.env` contains:

```env
OPENAI_API_KEY=your-openai-api-key-here
OPENAI_BASE_URL=https://api.openai.com/v1
ROUTIIUM_BASE=http://127.0.0.1:8099
MODEL=gpt-4.1-nano
```

## Running Tests

### Option 1: Full Automated Setup (Recommended)

```bash
./setup_and_test.sh
```

- Builds everything from scratch
- Starts server in background
- Runs all tests
- Auto-cleanup on exit

### Option 2: Quick Run (Server Already Running)

If routiium is already running:

```bash
ROUTIIUM_TEST_USE_EXTERNAL=1 ./run_tests.sh
```

This skips server startup and runs tests immediately.

### Option 3: Manual Control

```bash
# Install uv
curl -LsSf https://astral.sh/uv/install.sh | sh

# Setup environment
uv venv
source .venv/bin/activate
uv pip install -e .

# Start server in another terminal
cd ..
cargo run --release

# Run tests
pytest tests/ -v -s
```

## Auto-Managed Server via pytest

The pytest suite can now launch and teardown routiium automatically. Just run:

```bash
cd python_tests
pytest tests/ -v -s
```

To force tests to target an already running server, set:

```bash
ROUTIIUM_TEST_USE_EXTERNAL=1 pytest tests/ -v -s
```

## Automated Chat CLI Smoke Test

Need a quick end-to-end sanity check for the lightweight chat client without running the full pytest suite? Use the helper script:

```bash
cd python_tests
./run_chat_cli_e2e.sh --message "Hello Routiium!" --model gpt-4.1-nano
```

The script bootstraps the Python environment, (optionally) builds Routiium, starts the proxy, feeds the prompt through `chat_cli.py`, and shuts everything down when done. Pass `--reuse-server` to target an already running proxy or `--transcript logs/chat_cli.txt` to archive the output.

## Running Specific Tests

```bash
# Single test
./run_tests.sh -k test_basic_chat_completion

# Test class
./run_tests.sh -k TestChatCompletions
./run_tests.sh -k TestResponsesAPI

# By feature
./run_tests.sh -k tool          # Tool calling tests
./run_tests.sh -k vision        # Vision/image tests
./run_tests.sh -k streaming     # All streaming tests

# Verbose output
./run_tests.sh -v -s

# Stop on first failure
./run_tests.sh -x

# Vision tests with appropriate model
MODEL=gpt-4o-mini ./run_tests.sh -k vision
```

## What Gets Tested

✓ **Chat Completions API (5 tests)**
  - Non-streaming requests
  - Streaming responses
  - System messages
  - Parameter handling (max_tokens, temperature)

✓ **Responses API (15 tests)**
  - Basic responses via /v1/responses endpoint
  - Streaming with SSE format
  - System messages and parameters
  - Metadata preservation
  - Error handling

✓ **Tool Calling / Function Calling (3 tests)**
  - Single tool definition and invocation
  - Multiple tools handling
  - Streaming with tool calls

✓ **Vision / Multimodal (4 tests)**
  - Image URL inputs
  - Base64-encoded images
  - Streaming with images
  - Combined vision + tools

✓ **Proxy Behavior (3 tests)**
  - Conversation ID handling
  - Error propagation
  - Edge case handling

✓ **Performance (3 tests)**
  - Response latency measurement
  - Time-to-first-token (TTFT) metrics
  - Responses API endpoint latency

**Total: 26 comprehensive integration tests**

## Expected Output

```
===================== test session starts =====================
platform darwin -- Python 3.11.0, pytest-7.4.3, pluggy-1.3.0
rootdir: /path/to/routiium/python_tests
collected 26 items

tests/test_routiium_integration.py::TestChatCompletions::test_basic_chat_completion PASSED [  4%]
✓ Chat completion response: Hello, World!

tests/test_routiium_integration.py::TestChatCompletions::test_streaming_chat_completion PASSED [  8%]
✓ Streaming chat completion: 15 chunks, content: Hello, World!

tests/test_routiium_integration.py::TestResponsesAPI::test_basic_responses_endpoint PASSED [ 12%]
✓ Responses API endpoint response: Hello, World!

tests/test_routiium_integration.py::TestResponsesAPI::test_responses_endpoint_with_tools PASSED [ 16%]
✓ Tool call detected: get_weather({"location": "Tokyo"})

tests/test_routiium_integration.py::TestResponsesAPI::test_responses_endpoint_with_vision PASSED [ 20%]
✓ Vision/image input test: The image shows a scenic nature boardwalk...

... (more tests)

===================== 26 passed in 92.34s =====================
```

## Troubleshooting

### Port Already in Use

```bash
# Kill existing process on port 8099
lsof -ti:8099 | xargs kill -9
```

### Server Won't Start

```bash
# Check server status
curl http://127.0.0.1:8099/status

# View server logs
cd .. && cargo run --release
```

### Import Errors

```bash
# Reinstall dependencies
rm -rf .venv
uv venv
source .venv/bin/activate
uv pip install -e .
```

### API Key Issues

```bash
# Verify API key is set
grep OPENAI_API_KEY ../.env

# Test API key directly
curl https://api.openai.com/v1/models \
  -H "Authorization: Bearer $OPENAI_API_KEY"
```

## Development Workflow

### Add New Test

1. Edit `tests/test_routiium_integration.py`
2. Add test method to appropriate class
3. Run: `./run_tests.sh -k your_new_test`

Example:

```python
def test_my_feature(self, routiium_client, test_model):
    """Test my new feature."""
    response = routiium_client.chat.completions.create(
        model=test_model,
        messages=[{"role": "user", "content": "test"}],
    )
    assert response.choices[0].message.content is not None
```

### Debug Failed Test

```bash
# Run with Python debugger
pytest tests/ -k test_name --pdb

# Extra verbose output
pytest tests/ -k test_name -vv -s

# Show local variables on failure
pytest tests/ -k test_name -l
```

### Performance Profiling

```bash
# Install profiling tools
uv pip install pytest-benchmark pytest-profiling

# Run with timing
pytest tests/ --durations=10
```

## CI/CD Integration

### GitHub Actions Example

```yaml
name: Python Tests

on: [push, pull_request]

jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3
      - uses: dtolnay/rust-toolchain@stable
      - name: Run tests
        run: cd python_tests && ./setup_and_test.sh
        env:
          OPENAI_API_KEY: ${{ secrets.OPENAI_API_KEY }}
```

## Next Steps

- Read full documentation: [README.md](README.md)
- **Responses API testing guide:** [docs/RESPONSES_API_TESTING.md](docs/RESPONSES_API_TESTING.md)
- **Tool calling & vision guide:** [docs/TOOL_AND_VISION_TESTING.md](docs/TOOL_AND_VISION_TESTING.md)
- **Test accuracy report:** [TEST_ACCURACY_REPORT.md](TEST_ACCURACY_REPORT.md)
- Review test code: [tests/test_routiium_integration.py](tests/test_routiium_integration.py)
- Check main project: [../README.md](../README.md)

## Support

- Issues: https://github.com/labiium/routiium/issues
- Discussions: https://github.com/labiium/routiium/discussions

## Time Estimates

| Task | Time |
|------|------|
| First-time setup | 3-5 minutes |
| Run all tests (26 tests) | 60-120 seconds |
| Run specific subset | 15-30 seconds |
| Add new test | 5-10 minutes |
| Debug failed test | 10-30 minutes |

## Cost Estimates

Using `gpt-4.1-nano` (default model):
- Full test suite: ~$0.08-0.21 per run
- Chat completions only: ~$0.02-0.05
- Responses API only: ~$0.04-0.10
- Vision tests only: ~$0.03-0.06

**Recommended:** Use `gpt-4.1-nano` for cost-effective testing.
