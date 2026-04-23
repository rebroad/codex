#!/usr/bin/env python3

from __future__ import annotations

import argparse
import datetime as dt
import html.parser
import json
import pathlib
import sys
import urllib.request


DEFAULT_URL = "https://developers.openai.com/api/docs/pricing"
DEFAULT_OUTPUT = pathlib.Path.home() / ".codex" / "model_pricing.json"
CREDITS_PER_USD = 25.0


class PricingHTMLTextExtractor(html.parser.HTMLParser):
    def __init__(self) -> None:
        super().__init__()
        self._ignored_tag: str | None = None
        self._parts: list[str] = []

    def handle_starttag(self, tag: str, attrs: list[tuple[str, str | None]]) -> None:
        if tag in {"script", "style"}:
            self._ignored_tag = tag
        else:
            self._parts.append(" ")

    def handle_endtag(self, tag: str) -> None:
        if self._ignored_tag == tag:
            self._ignored_tag = None
        else:
            self._parts.append(" ")

    def handle_data(self, data: str) -> None:
        if self._ignored_tag is None:
            self._parts.append(data)

    def text(self) -> str:
        return " ".join(" ".join(self._parts).split())


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Fetch OpenAI pricing and write ~/.codex/model_pricing.json.",
    )
    parser.add_argument("--url", default=DEFAULT_URL, help="Pricing page URL to fetch.")
    parser.add_argument(
        "--output",
        default=str(DEFAULT_OUTPUT),
        help="Where to write the pricing JSON.",
    )
    parser.add_argument(
        "--html-file",
        help="Parse pricing HTML from a local file instead of fetching the URL.",
    )
    return parser.parse_args()


def fetch_html(url: str) -> str:
    request = urllib.request.Request(
        url,
        headers={"User-Agent": "codex-model-pricing-updater/1.0"},
    )
    with urllib.request.urlopen(request) as response:
        return response.read().decode("utf-8")


def normalize_html(html: str) -> str:
    parser = PricingHTMLTextExtractor()
    parser.feed(html)
    return parser.text()


def parse_models_from_text(text: str) -> dict[str, dict[str, float]]:
    tokens = text.split()
    models: dict[str, dict[str, float]] = {}

    for index in range(len(tokens) - 3):
        model = tokens[index]
        if not looks_like_priced_model_token(model):
            continue

        input_usd = parse_usd_token(tokens[index + 1])
        cached_input_usd = parse_usd_token(tokens[index + 2])
        output_usd = parse_usd_token(tokens[index + 3])
        if input_usd is None or cached_input_usd is None or output_usd is None:
            continue

        models.setdefault(
            model,
            {
                "input_credits_per_million": input_usd * CREDITS_PER_USD,
                "cached_input_credits_per_million": cached_input_usd * CREDITS_PER_USD,
                "output_credits_per_million": output_usd * CREDITS_PER_USD,
            },
        )

    return models


def looks_like_priced_model_token(token: str) -> bool:
    return token.startswith("gpt-") and all(
        char.isalnum() or char in {"-", "."} for char in token
    )


def parse_usd_token(token: str) -> float | None:
    if not token.startswith("$"):
        return None
    try:
        return float(token[1:])
    except ValueError:
        return None


def compatibility_aliases(models: dict[str, dict[str, float]]) -> dict[str, str]:
    aliases: dict[str, str] = {}
    if "gpt-5.4-mini" in models:
        aliases["gpt-5.1-codex-mini"] = "gpt-5.4-mini"
        aliases["gpt-5-codex-mini"] = "gpt-5.4-mini"
    if "gpt-5.3-codex" in models:
        aliases["gpt-5.2-codex"] = "gpt-5.3-codex"
        aliases["gpt-5.2"] = "gpt-5.3-codex"
    return aliases


def current_timestamp() -> str:
    return dt.datetime.now(dt.UTC).replace(microsecond=0).isoformat().replace("+00:00", "Z")


def main() -> int:
    args = parse_args()
    if args.html_file:
        html = pathlib.Path(args.html_file).read_text(encoding="utf-8")
    else:
        html = fetch_html(args.url)

    normalized_text = normalize_html(html)
    models = parse_models_from_text(normalized_text)
    if not models:
        print("No pricing rows were parsed from the supplied HTML.", file=sys.stderr)
        return 1

    payload = {
        "version": 1,
        "default_model": "gpt-5.3-codex",
        "source_url": args.url,
        "updated_at": current_timestamp(),
        "credits_per_usd": CREDITS_PER_USD,
        "models": models,
        "aliases": compatibility_aliases(models),
    }

    output_path = pathlib.Path(args.output).expanduser()
    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(f"Wrote {len(models)} pricing rows to {output_path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
