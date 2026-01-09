import json
import os
from pathlib import Path

import pytest
import requests


def _post_chat(base_url, payload, token=None, timeout=120):
    headers = {}
    if token:
        headers["Authorization"] = f"Bearer {token}"
    resp = requests.post(
        f"{base_url}/v1/chat/completions",
        json=payload,
        headers=headers,
        timeout=timeout,
    )
    if resp.status_code != 200:
        pytest.fail(
            f"POST {base_url}/v1/chat/completions failed: {resp.status_code} {resp.text}"
        )
    return resp.json()


def _load_system_prompt_config():
    path = os.getenv("ROUTIIUM_TEST_SYSTEM_PROMPT_PATH")
    if not path:
        path = Path(__file__).resolve().parents[1] / "system_prompt.json"
    path = Path(path)
    if not path.is_file():
        return None
    with path.open("r", encoding="utf-8") as fh:
        return json.load(fh)


def _prompt_for_model(config, model, api):
    if not config or not config.get("enabled", True):
        return None, "prepend"
    mode = config.get("injection_mode", "prepend")
    per_model = config.get("per_model", {})
    if model in per_model:
        return per_model[model], mode
    per_api = config.get("per_api", {})
    if api in per_api:
        return per_api[api], mode
    return config.get("global"), mode


def _apply_prompt(messages, prompt, mode):
    if not prompt:
        return list(messages)
    system_message = {"role": "system", "content": prompt}
    if mode == "append":
        idx = None
        for i, msg in enumerate(messages):
            if msg.get("role") == "system":
                idx = i
        if idx is not None:
            return messages[: idx + 1] + [system_message] + messages[idx + 1 :]
        return list(messages) + [system_message]
    if mode == "replace":
        return [system_message] + [m for m in messages if m.get("role") != "system"]
    return [system_message] + list(messages)


def _normalize_message(resp):
    choice = resp.get("choices", [{}])[0]
    return choice.get("message", {})


def _is_truthy(value):
    if not value:
        return False
    return value.strip().lower() in {"1", "true", "yes", "on"}


def _final_text(msg):
    content = msg.get("content")
    if isinstance(content, str) and content.strip():
        return content
    reasoning = msg.get("reasoning_content")
    if isinstance(reasoning, str) and reasoning.strip():
        return reasoning
    return ""


def test_vllm_chat_forwarding_and_reasoning_content():
    if _is_truthy(os.getenv("VLLM_E2E_DISABLE")):
        pytest.skip("VLLM_E2E_DISABLE set")

    routiium_base = os.getenv("ROUTIIUM_BASE", "http://127.0.0.1:8099")
    vllm_base = os.getenv("VLLM_BASE", "http://100.69.61.40:8000")
    vllm_model = os.getenv("VLLM_MODEL", "nemotron-nano-30b-fp8")
    route_model = os.getenv("VLLM_ROUTE_MODEL", vllm_model)
    vllm_token = os.getenv("VLLM_API_KEY")
    routiium_token = os.getenv("ROUTIIUM_ACCESS_TOKEN") or os.getenv("OPENAI_API_KEY")

    config = _load_system_prompt_config()
    prompt, mode = _prompt_for_model(config, route_model, "chat")

    base_messages = [{"role": "user", "content": "Write a short 4-line poem."}]
    direct_messages = _apply_prompt(base_messages, prompt, mode)

    common_params = {
        "temperature": 0,
        "seed": 123,
        "max_tokens": 128,
        "top_k": 40,
        "repetition_penalty": 1.1,
        "chat_template_kwargs": {"enable_thinking": False},
    }

    direct_payload = {
        "model": vllm_model,
        "messages": direct_messages,
        **common_params,
    }
    routed_payload = {
        "model": route_model,
        "messages": base_messages,
        **common_params,
    }

    direct_resp = _post_chat(vllm_base, direct_payload, token=vllm_token)
    routed_resp = _post_chat(routiium_base, routed_payload, token=routiium_token)

    direct_msg = _normalize_message(direct_resp)
    routed_msg = _normalize_message(routed_resp)

    strict_match = _is_truthy(os.getenv("VLLM_STRICT_MATCH"))
    require_reasoning = _is_truthy(os.getenv("VLLM_REQUIRE_REASONING"))

    direct_reasoning = direct_msg.get("reasoning_content")
    routed_reasoning = routed_msg.get("reasoning_content")

    if require_reasoning and not direct_reasoning:
        pytest.fail("Direct vLLM response missing reasoning_content")
    if direct_reasoning:
        assert routed_reasoning, "Routed response missing reasoning_content"
        if strict_match:
            assert routed_reasoning == direct_reasoning

    direct_text = _final_text(direct_msg)
    routed_text = _final_text(routed_msg)
    assert direct_text, "Direct response missing final text"
    assert routed_text, "Routed response missing final text"
    if strict_match:
        assert routed_text == direct_text

    direct_usage = direct_resp.get("usage")
    routed_usage = routed_resp.get("usage")
    if direct_usage and routed_usage and common_params.get("max_tokens"):
        max_tokens = common_params["max_tokens"]
        assert direct_usage.get("completion_tokens", 0) <= max_tokens
        assert routed_usage.get("completion_tokens", 0) <= max_tokens
