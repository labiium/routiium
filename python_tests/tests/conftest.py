"""
Pytest configuration and shared fixtures for routiium integration tests.

This module sets up authentication tokens for testing in managed mode.
"""

import json
import os
import socket
import subprocess
import tempfile
import time
from pathlib import Path

import pytest
import requests
from dotenv import load_dotenv

# Load environment variables
load_dotenv(dotenv_path=os.path.join(os.path.dirname(__file__), "../../.env"))

DEFAULT_VLLM_BASE = "http://100.69.61.40:8000"
DEFAULT_VLLM_MODEL = "nemotron-nano-30b-fp8"


def _set_default_env(key: str, value: str) -> None:
    if not os.getenv(key):
        os.environ[key] = value


_set_default_env("VLLM_BASE", DEFAULT_VLLM_BASE)
_set_default_env("VLLM_MODEL", DEFAULT_VLLM_MODEL)
if not os.getenv("VLLM_ROUTE_MODEL"):
    os.environ["VLLM_ROUTE_MODEL"] = f"vllm-{os.environ['VLLM_MODEL']}"
if not os.getenv("ROUTIIUM_BACKENDS"):
    vllm_base = os.environ["VLLM_BASE"].rstrip("/")
    os.environ["ROUTIIUM_BACKENDS"] = f"prefix=vllm-,base={vllm_base}/v1,mode=chat"


def _is_truthy(value: str | None) -> bool:
    if not value:
        return False
    return value.strip().lower() in {"1", "true", "yes", "on"}


def _pick_free_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def _wait_for_status(base_url: str, timeout_seconds: int) -> bool:
    deadline = time.time() + timeout_seconds
    while time.time() < deadline:
        try:
            resp = requests.get(f"{base_url}/status", timeout=1)
            if resp.status_code == 200:
                return True
        except Exception:
            pass
        time.sleep(0.5)
    return False


@pytest.fixture(scope="session", autouse=True)
def routiium_server():
    use_external = _is_truthy(os.getenv("ROUTIIUM_TEST_USE_EXTERNAL"))
    if use_external:
        base_url = os.getenv("ROUTIIUM_BASE", "http://127.0.0.1:8099")
        if not _wait_for_status(base_url, 5):
            pytest.exit(
                f"ROUTIIUM_TEST_USE_EXTERNAL=1 but {base_url}/status is not reachable"
            )
        yield base_url
        return

    project_root = Path(__file__).resolve().parents[2]
    python_tests_dir = project_root / "python_tests"
    binary_path = project_root / "target" / "release" / "routiium"

    if not binary_path.exists():
        build = subprocess.run(
            ["cargo", "build", "--release"], cwd=str(project_root), check=False
        )
        if build.returncode != 0:
            pytest.exit("Failed to build routiium binary for tests")

    bind_addr_env = os.getenv("ROUTIIUM_TEST_BIND_ADDR")
    if bind_addr_env and ":" in bind_addr_env:
        bind_addr = bind_addr_env
        port = int(bind_addr_env.rsplit(":", 1)[-1])
    else:
        port = int(os.getenv("ROUTIIUM_TEST_PORT") or _pick_free_port())
        bind_addr = bind_addr_env or f"127.0.0.1:{port}"

    base_url = f"http://127.0.0.1:{port}"
    os.environ["ROUTIIUM_BASE"] = base_url

    temp_dir = tempfile.TemporaryDirectory(prefix="routiium-test-")
    env = os.environ.copy()
    env["BIND_ADDR"] = bind_addr
    env["ROUTIIUM_SLED_PATH"] = os.path.join(temp_dir.name, "keys.db")

    cli_args = []
    router_config_path = python_tests_dir / "router_aliases.json"
    if router_config_path.is_file():
        try:
            with router_config_path.open("r", encoding="utf-8") as fh:
                router_aliases = json.load(fh)
        except Exception:
            router_aliases = {}

        vllm_route_model = os.getenv("VLLM_ROUTE_MODEL")
        vllm_model = os.getenv("VLLM_MODEL")
        vllm_base = os.getenv("VLLM_BASE")
        if vllm_route_model and vllm_model and vllm_base:
            if vllm_route_model not in router_aliases:
                router_aliases[vllm_route_model] = {
                    "base_url": f"{vllm_base.rstrip('/')}/v1",
                    "mode": "chat",
                    "model_id": vllm_model,
                }

        router_tmp_path = Path(temp_dir.name) / "router_aliases.json"
        with router_tmp_path.open("w", encoding="utf-8") as fh:
            json.dump(router_aliases, fh)
        cli_args.append(f"--router-config={router_tmp_path}")

    mcp_config_path = python_tests_dir / "mcp" / "mcp.json"
    if mcp_config_path.is_file():
        cli_args.append(f"--mcp-config={mcp_config_path}")

    system_prompt_config_path = python_tests_dir / "system_prompt.json"
    if system_prompt_config_path.is_file():
        cli_args.append(f"--system-prompt-config={system_prompt_config_path}")
        os.environ["ROUTIIUM_TEST_SYSTEM_PROMPT_PATH"] = str(system_prompt_config_path)

    log_path = os.path.join(temp_dir.name, "routiium-test.log")
    log_file = open(log_path, "w", encoding="utf-8")
    proc = subprocess.Popen(
        [str(binary_path), *cli_args],
        cwd=str(project_root),
        env=env,
        stdout=log_file,
        stderr=subprocess.STDOUT,
    )

    if not _wait_for_status(base_url, 30):
        proc.terminate()
        try:
            proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            proc.kill()
        log_file.close()
        temp_dir.cleanup()
        pytest.exit(f"routiium failed to start; see log at {log_path}")

    try:
        yield base_url
    finally:
        proc.terminate()
        try:
            proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            proc.kill()
        log_file.close()
        temp_dir.cleanup()


@pytest.fixture(scope="session", autouse=True)
def setup_test_api_key(routiium_server):
    """
    Generate a test API key for use in managed authentication mode.

    This fixture runs once per test session and generates a temporary
    access token that all tests can use. The token is stored in an
    environment variable.
    """
    base_url = os.getenv("ROUTIIUM_BASE", "http://127.0.0.1:8099")

    # Generate a temporary access token
    try:
        response = requests.post(
            f"{base_url}/keys/generate",
            json={"label": "pytest-session", "ttl_seconds": 3600},
            timeout=5,
        )
        response.raise_for_status()

        key_data = response.json()
        access_token = key_data.get("token")

        if not access_token:
            pytest.exit(
                f"Failed to generate test access token: no token in response: {key_data}"
            )

        # Store the access token for tests to use
        os.environ["ROUTIIUM_ACCESS_TOKEN"] = access_token
        print(f"\n✓ Generated test access token: {access_token[:20]}...")

        yield access_token

    except Exception as e:
        pytest.exit(f"Failed to generate test access token: {e}")
