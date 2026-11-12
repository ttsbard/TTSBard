"""Unit tests for scripts/validate-model-catalog.py."""

from __future__ import annotations

import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
SCRIPT_PATH = ROOT / "scripts" / "validate-model-catalog.py"

_SPEC = importlib.util.spec_from_file_location("validate_model_catalog", SCRIPT_PATH)
assert _SPEC is not None and _SPEC.loader is not None
_MODULE = importlib.util.module_from_spec(_SPEC)
_SPEC.loader.exec_module(_MODULE)

CatalogValidationError = _MODULE.CatalogValidationError
validate_catalog = _MODULE.validate_catalog


def _inline_file(path: str, content: str) -> dict[str, object]:
    encoded = content.encode("utf-8")
    return {
        "path": path,
        "content": content,
        "sha256": hashlib.sha256(encoded).hexdigest(),
        "size_bytes": len(encoded),
    }


def _valid_ocr_manifest(package_id: str = "eslav-ppocrv5-mobile") -> str:
    return json.dumps(
        {
            "id": package_id,
            "display_name": "eSlav PP-OCRv5 Mobile",
            "languages": ["ru", "en"],
            "family": "pp_ocr_v5",
            "det_file": "det.onnx",
            "rec_file": "rec.onnx",
            "dict_file": "dict.txt",
        }
    )


def _base_catalog() -> dict[str, object]:
    return {
        "schema_version": 1,
        "published_at": "2026-11-11",
        "packages": [
            {
                "id": "eslav-ppocrv5-mobile",
                "category": "ocr",
                "name": "eSlav PP-OCRv5 Mobile (RU / EN)",
                "version": "5.0",
                "source_url": "https://www.modelscope.cn/models/RapidAI/RapidOCR",
                "license": "Apache-2.0",
                "recommended": True,
                "available": True,
                "files": [
                    _inline_file("ocr/eslav-ppocrv5-mobile/manifest.json", _valid_ocr_manifest()),
                    {
                        "path": "ocr/eslav-ppocrv5-mobile/det.onnx",
                        "url": "https://example.com/det.onnx",
                        "sha256": "a" * 64,
                        "size_bytes": 123,
                    },
                ],
            },
            {
                "id": "xvibe-ru-v0.1.0",
                "category": "pocket",
                "name": "xVibe Russian",
                "version": "0.1.0",
                "source_url": "https://huggingface.co/genvoice/xVibePocketTTS",
                "license": None,
                "available": False,
                "unavailable_reason": "License is not confirmed.",
                "files": [],
            },
        ],
    }


class ValidateModelCatalogTests(unittest.TestCase):
    def _write_catalog(self, tmp: Path, payload: dict[str, object], *, suffix: bytes = b"\n", bom: bool = False) -> Path:
        target = tmp / "model-catalog.json"
        raw = json.dumps(payload, indent=2).encode("utf-8") + suffix
        if bom:
            raw = b"\xef\xbb\xbf" + raw
        target.write_bytes(raw)
        return target

    def test_repository_catalog_passes_offline_validation(self) -> None:
        summary = validate_catalog(ROOT / "catalog" / "model-catalog.json")
        self.assertGreaterEqual(summary["packages"], 1)
        self.assertGreater(summary["downloadable"], 0)

    def test_minimal_valid_catalog_passes(self) -> None:
        with tempfile.TemporaryDirectory() as tmp_dir:
            path = self._write_catalog(Path(tmp_dir), _base_catalog())
            summary = validate_catalog(path)
            self.assertEqual(summary["packages"], 2)
            self.assertEqual(summary["inline_content"], 1)
            self.assertEqual(summary["downloadable"], 1)

    def test_rejects_bom_and_missing_trailing_newline(self) -> None:
        with tempfile.TemporaryDirectory() as tmp_dir:
            tmp = Path(tmp_dir)
            with self.assertRaisesRegex(CatalogValidationError, "BOM"):
                validate_catalog(self._write_catalog(tmp, _base_catalog(), bom=True))
            with self.assertRaisesRegex(CatalogValidationError, "newline"):
                validate_catalog(self._write_catalog(tmp, _base_catalog(), suffix=b""))

    def test_rejects_invalid_date_and_duplicate_id(self) -> None:
        with tempfile.TemporaryDirectory() as tmp_dir:
            tmp = Path(tmp_dir)
            bad_date = _base_catalog()
            bad_date["published_at"] = "2026-02-30"
            with self.assertRaisesRegex(CatalogValidationError, "published_at"):
                validate_catalog(self._write_catalog(tmp, bad_date))

            dup = _base_catalog()
            dup["packages"].append(dict(dup["packages"][1]))  # type: ignore[union-attr]
            with self.assertRaisesRegex(CatalogValidationError, "duplicate package id"):
                validate_catalog(self._write_catalog(tmp, dup))

            upper = _base_catalog()
            upper["packages"].append(dict(upper["packages"][1], id="BadUpper"))  # type: ignore[union-attr]
            with self.assertRaisesRegex(CatalogValidationError, "invalid package id"):
                validate_catalog(self._write_catalog(tmp, upper))

    def test_rejects_unavailable_with_files_and_bad_inline_hash(self) -> None:
        with tempfile.TemporaryDirectory() as tmp_dir:
            tmp = Path(tmp_dir)
            bad_unavail = _base_catalog()
            bad_unavail["packages"][1]["files"] = [_inline_file("pocket/a/b.txt", "hi")]  # type: ignore[index]
            with self.assertRaisesRegex(CatalogValidationError, "must not list files"):
                validate_catalog(self._write_catalog(tmp, bad_unavail))

            bad_hash = _base_catalog()
            bad_hash["packages"][0]["files"][0]["sha256"] = "0" * 64  # type: ignore[index]
            with self.assertRaisesRegex(CatalogValidationError, "sha256 mismatch"):
                validate_catalog(self._write_catalog(tmp, bad_hash))

    def test_rejects_conflicting_shared_paths_and_unsafe_paths(self) -> None:
        with tempfile.TemporaryDirectory() as tmp_dir:
            tmp = Path(tmp_dir)
            shared_conflict = _base_catalog()
            shared_conflict["packages"].append(  # type: ignore[union-attr]
                {
                    "id": "other-ocr",
                    "category": "ocr",
                    "name": "Other OCR",
                    "version": "1.0",
                    "available": True,
                    "files": [
                        {
                            "path": "ocr/eslav-ppocrv5-mobile/det.onnx",
                            "url": "https://example.com/other-det.onnx",
                            "sha256": "b" * 64,
                            "size_bytes": 123,
                        },
                    ],
                }
            )
            with self.assertRaisesRegex(CatalogValidationError, "conflicting shared path"):
                validate_catalog(self._write_catalog(tmp, shared_conflict))

            manifest_mismatch = _base_catalog()
            manifest_mismatch["packages"].append(  # type: ignore[union-attr]
                {
                    "id": "other-ocr",
                    "category": "ocr",
                    "name": "Other OCR",
                    "version": "1.0",
                    "available": True,
                    "files": [
                        _inline_file("ocr/other-ocr/manifest.json", _valid_ocr_manifest("wrong-id")),
                    ],
                }
            )
            with self.assertRaisesRegex(CatalogValidationError, "manifest id"):
                validate_catalog(self._write_catalog(tmp, manifest_mismatch))

            bad_path = _base_catalog()
            bad_path["packages"][0]["files"][1]["path"] = "ocr/../escape/det.onnx"  # type: ignore[index]
            with self.assertRaisesRegex(CatalogValidationError, "invalid path segment"):
                validate_catalog(self._write_catalog(tmp, bad_path))


if __name__ == "__main__":
    unittest.main()
