"""Validate catalog/model-catalog.json against the v1 schema and invariants."""

from __future__ import annotations

import argparse
from concurrent.futures import ThreadPoolExecutor
from datetime import date
import hashlib
import json
from pathlib import Path
import re
import sys
import time
from typing import Any
import urllib.error
import urllib.request

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_CATALOG = ROOT / "catalog" / "model-catalog.json"

ALLOWED_TOP_KEYS = {"schema_version", "published_at", "packages"}
ALLOWED_PACKAGE_KEYS = {
    "id",
    "category",
    "name",
    "version",
    "source_url",
    "license",
    "recommended",
    "available",
    "unavailable_reason",
    "files",
}
REQUIRED_PACKAGE_KEYS = {"id", "category", "name", "version", "available", "files"}
ALLOWED_CATEGORIES = {"ruaccent", "ocr", "pocket"}
PACKAGE_ID_RE = re.compile(r"^[a-z0-9][a-z0-9._-]*$")
DATE_RE = re.compile(r"^\d{4}-\d{2}-\d{2}$")
SHA256_RE = re.compile(r"^[0-9a-f]{64}$")
WINDOWS_RESERVED_NAMES = {
    "con",
    "prn",
    "aux",
    "nul",
    *(f"com{i}" for i in range(1, 10)),
    *(f"lpt{i}" for i in range(1, 10)),
}
INVALID_WINDOWS_CHARS = set('<>:"|?*')


class CatalogValidationError(ValueError):
    """Raised when the catalog violates schema or integrity rules."""


def _require(condition: bool, message: str) -> None:
    if not condition:
        raise CatalogValidationError(message)


def _is_non_empty_str(value: Any) -> bool:
    return isinstance(value, str) and bool(value.strip())


def _validate_relative_path(path: Any, category: str, package_id: str) -> str:
    _require(isinstance(path, str) and path != "", f"{package_id}: file path must be non-empty string")
    _require(
        not path.startswith("/") and "\\" not in path,
        f"{package_id}: path must use relative POSIX separators: {path!r}",
    )
    _require(
        path.startswith(f"{category}/"),
        f"{package_id}: path {path!r} must start with '{category}/'",
    )
    segments = path.split("/")
    for seg in segments:
        _require(seg != "" and seg not in {".", ".."}, f"{package_id}: invalid path segment in {path!r}")
        _require(
            not seg.endswith(" ") and not seg.endswith("."),
            f"{package_id}: Windows-unsafe trailing char in segment {seg!r} of {path!r}",
        )
        _require(
            not any(ch in INVALID_WINDOWS_CHARS or ord(ch) < 32 for ch in seg),
            f"{package_id}: Windows-unsafe char in {path!r}",
        )
        stem = seg.split(".", 1)[0].lower()
        _require(stem not in WINDOWS_RESERVED_NAMES, f"{package_id}: Windows reserved name in {path!r}")
    return path.lower()


def _validate_inline_json(path: str, content: str, category: str, package_id: str) -> None:
    if not path.endswith(".json"):
        return
    try:
        parsed = json.loads(content)
    except json.JSONDecodeError as exc:
        raise CatalogValidationError(f"{package_id}: invalid inline JSON in {path!r}: {exc}") from exc

    if category == "ocr" and path.endswith("/manifest.json"):
        _require(isinstance(parsed, dict), f"{package_id}: OCR manifest must be a JSON object")
        for key in ("id", "display_name", "family", "det_file", "rec_file", "dict_file"):
            _require(_is_non_empty_str(parsed.get(key)), f"{package_id}: OCR manifest missing '{key}'")
        _require(
            parsed["id"] == package_id,
            f"{package_id}: OCR manifest id {parsed['id']!r} does not match package id",
        )
        langs = parsed.get("languages")
        _require(
            isinstance(langs, list) and bool(langs) and all(_is_non_empty_str(x) for x in langs),
            f"{package_id}: OCR manifest 'languages' must be a non-empty list of strings",
        )


def validate_catalog(catalog_path: Path) -> dict[str, Any]:
    """Validate a catalog file offline and return summary statistics."""
    _require(catalog_path.is_file(), f"Catalog file not found: {catalog_path}")
    raw = catalog_path.read_bytes()
    _require(not raw.startswith(b"\xef\xbb\xbf"), "catalog must not have a UTF-8 BOM")
    _require(raw.endswith(b"\n"), "catalog must end with a newline")

    try:
        data = json.loads(raw.decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise CatalogValidationError(f"Catalog is not valid UTF-8 JSON: {exc}") from exc

    _require(isinstance(data, dict), "catalog root must be a JSON object")
    _require(
        set(data) == ALLOWED_TOP_KEYS,
        f"unexpected top-level keys: {sorted(set(data))} != {sorted(ALLOWED_TOP_KEYS)}",
    )
    _require(
        isinstance(data["schema_version"], int)
        and not isinstance(data["schema_version"], bool)
        and data["schema_version"] == 1,
        f"unsupported schema_version: {data['schema_version']!r}",
    )

    published_at = data["published_at"]
    _require(
        isinstance(published_at, str) and bool(DATE_RE.fullmatch(published_at)),
        f"published_at must be YYYY-MM-DD, got {published_at!r}",
    )
    try:
        date.fromisoformat(published_at)
    except ValueError as exc:
        raise CatalogValidationError(f"published_at is not a valid calendar date: {published_at!r}") from exc

    packages = data["packages"]
    _require(isinstance(packages, list) and bool(packages), "packages must be a non-empty list")

    seen_ids: set[str] = set()
    seen_paths: dict[str, tuple[str, int, str]] = {}
    all_file_segments: set[tuple[str, ...]] = set()
    all_dir_segments: set[tuple[str, ...]] = set()
    downloadable = 0
    content_files = 0
    total_bytes = 0
    unique_urls: list[tuple[str, str]] = []
    seen_url_set: set[str] = set()

    for idx, package in enumerate(packages):
        _require(isinstance(package, dict), f"packages[{idx}] must be an object")
        extra_keys = set(package) - ALLOWED_PACKAGE_KEYS
        _require(not extra_keys, f"packages[{idx}]: unexpected keys {sorted(extra_keys)}")
        missing_keys = REQUIRED_PACKAGE_KEYS - set(package)
        _require(not missing_keys, f"packages[{idx}]: missing keys {sorted(missing_keys)}")

        package_id = package["id"]
        _require(
            isinstance(package_id, str) and bool(PACKAGE_ID_RE.fullmatch(package_id)),
            f"packages[{idx}]: invalid package id {package_id!r}",
        )
        id_key = package_id.lower()
        _require(id_key not in seen_ids, f"duplicate package id (case-insensitive): {package_id!r}")
        seen_ids.add(id_key)

        category = package["category"]
        _require(category in ALLOWED_CATEGORIES, f"{package_id}: invalid category {category!r}")
        _require(_is_non_empty_str(package["name"]), f"{package_id}: name must be non-empty")
        _require(_is_non_empty_str(package["version"]), f"{package_id}: version must be non-empty")

        source_url = package.get("source_url")
        if source_url is not None:
            _require(
                isinstance(source_url, str) and source_url.startswith("https://"),
                f"{package_id}: source_url must start with https://, got {source_url!r}",
            )

        license_value = package.get("license")
        if license_value is not None:
            _require(_is_non_empty_str(license_value), f"{package_id}: license cannot be empty")

        if "recommended" in package and package["recommended"] is not None:
            _require(
                isinstance(package["recommended"], bool),
                f"{package_id}: recommended must be a boolean",
            )

        available = package["available"]
        _require(isinstance(available, bool), f"{package_id}: available must be a boolean")

        files = package["files"]
        _require(isinstance(files, list), f"{package_id}: files must be a list")

        if available:
            _require(bool(files), f"available package {package_id} has no files")
            _require(
                package.get("unavailable_reason") is None,
                f"available package {package_id} must not have unavailable_reason",
            )
        else:
            _require(
                _is_non_empty_str(package.get("unavailable_reason")),
                f"unavailable package {package_id} must explain unavailable_reason",
            )
            _require(files == [], f"unavailable package {package_id} must not list files")
            continue

        pkg_paths: set[str] = set()
        for item_idx, item in enumerate(files):
            _require(isinstance(item, dict), f"{package_id}.files[{item_idx}] must be an object")
            item_keys = set(item)
            _require(
                item_keys
                in (
                    {"path", "url", "sha256", "size_bytes"},
                    {"path", "content", "sha256", "size_bytes"},
                ),
                f"{package_id}.files[{item_idx}]: unexpected key set {sorted(item_keys)}",
            )

            path = item["path"]
            path_key = _validate_relative_path(path, category, package_id)
            _require(path_key not in pkg_paths, f"{package_id}: duplicate path {path!r}")
            pkg_paths.add(path_key)

            segments = tuple(path_key.split("/"))
            for prefix_len in range(1, len(segments)):
                dir_prefix = segments[:prefix_len]
                _require(
                    dir_prefix not in all_file_segments,
                    f"{package_id}: file/directory collision at {path!r}",
                )
                all_dir_segments.add(dir_prefix)
            _require(
                segments not in all_dir_segments,
                f"{package_id}: file/directory collision at {path!r}",
            )
            all_file_segments.add(segments)

            sha256 = item["sha256"]
            _require(
                isinstance(sha256, str) and bool(SHA256_RE.fullmatch(sha256)),
                f"{package_id}: invalid sha256 {sha256!r} for {path!r}",
            )
            size_bytes = item["size_bytes"]
            _require(
                isinstance(size_bytes, int)
                and not isinstance(size_bytes, bool)
                and size_bytes > 0,
                f"{package_id}: invalid size_bytes {size_bytes!r} for {path!r}",
            )

            if "url" in item:
                url = item["url"]
                _require(
                    isinstance(url, str) and url.startswith("https://"),
                    f"{package_id}: url must start with https:// for {path!r}",
                )
                source_sig = url
                downloadable += 1
                if url not in seen_url_set:
                    seen_url_set.add(url)
                    unique_urls.append((path, url))
            else:
                content = item["content"]
                _require(isinstance(content, str), f"{package_id}: content must be string for {path!r}")
                encoded = content.encode("utf-8")
                _require(
                    len(encoded) == size_bytes,
                    f"{package_id}: size_bytes mismatch for inline {path!r} ({len(encoded)} != {size_bytes})",
                )
                actual_hash = hashlib.sha256(encoded).hexdigest()
                _require(
                    actual_hash == sha256,
                    f"{package_id}: sha256 mismatch for inline {path!r}",
                )
                _validate_inline_json(path, content, category, package_id)
                source_sig = content
                content_files += 1

            signature = (sha256, size_bytes, source_sig)
            if path_key in seen_paths:
                _require(
                    seen_paths[path_key] == signature,
                    f"{package_id}: conflicting shared path {path!r}",
                )
            else:
                seen_paths[path_key] = signature
                total_bytes += size_bytes

    return {
        "packages": len(packages),
        "downloadable": downloadable,
        "inline_content": content_files,
        "total_declared_bytes": total_bytes,
        "catalog_bytes": len(raw),
        "catalog_sha256": hashlib.sha256(raw).hexdigest(),
        "unique_urls": unique_urls,
    }


def _probe_single_url(entry: tuple[str, str], timeout: float) -> str:
    rel_path, url = entry
    headers = {"User-Agent": "ttsbard-model-catalog-verify/1"}
    last_error: Exception | None = None
    for attempt in range(4):
        try:
            request = urllib.request.Request(url, headers=headers, method="HEAD")
            with urllib.request.urlopen(request, timeout=timeout) as response:
                status = response.status
            if status == 200:
                last_error = None
                break
            raise RuntimeError(f"HTTP {status}")
        except urllib.error.HTTPError as exc:
            # Some mirrors reject HEAD with 405; probe first byte via GET.
            if exc.code == 405:
                probe = urllib.request.Request(
                    url,
                    headers={**headers, "Range": "bytes=0-0"},
                    method="GET",
                )
                try:
                    with urllib.request.urlopen(probe, timeout=timeout) as response:
                        if response.status in (200, 206):
                            last_error = None
                            break
                        raise RuntimeError(f"HTTP {response.status}")
                except Exception as probe_exc:
                    last_error = probe_exc
            else:
                last_error = exc
        except Exception as exc:
            last_error = exc

        if attempt < 3:
            time.sleep(1.5 * (attempt + 1))

    if last_error is not None:
        raise CatalogValidationError(f"URL check failed for {rel_path} ({url}): {last_error}")
    return f"HEAD 200 {rel_path}"


def verify_urls(unique_urls: list[tuple[str, str]], timeout: float = 30.0) -> None:
    """Verify that every catalogued URL responds with HTTP 200 without auth."""
    with ThreadPoolExecutor(max_workers=6) as pool:
        for line in pool.map(lambda entry: _probe_single_url(entry, timeout), unique_urls):
            print(line, flush=True)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Validate TTSBard model catalog JSON.")
    parser.add_argument(
        "--catalog",
        type=Path,
        default=DEFAULT_CATALOG,
        help=f"Path to model-catalog.json (default: {DEFAULT_CATALOG})",
    )
    parser.add_argument(
        "--check-urls",
        action="store_true",
        help="Verify that all remote file URLs return HTTP 200 without authentication.",
    )
    args = parser.parse_args(argv)

    try:
        summary = validate_catalog(args.catalog.resolve())
        if args.check_urls:
            verify_urls(summary["unique_urls"])
    except CatalogValidationError as exc:
        print(f"INVALID: {exc}", file=sys.stderr)
        return 1

    mib = summary["total_declared_bytes"] / (1024 * 1024)
    print(f"packages={summary['packages']}")
    print(f"downloadable={summary['downloadable']} inline_content={summary['inline_content']}")
    print(f"total_declared_bytes={summary['total_declared_bytes']} ({mib:.2f} MiB)")
    print(f"catalog_bytes={summary['catalog_bytes']} sha256={summary['catalog_sha256']}")
    print("VALID")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
