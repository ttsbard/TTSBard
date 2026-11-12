**English | [Русский](../../ru/user/ocr.md)**

# OCR: models and screen recognition

OCR recognizes a selected screen area once and sends text to **Incoming**.
Recognition is local and does not translate. Speech uses your selected TTS
provider, which may be a cloud service. Neither LunaTranslator nor the incoming
HTTP server is required. For text extracted by LunaTranslator, use
[its integration](../integrations/lunatranslator/README.md).

## Required package

TTSBard supports **PP-OCRv5** ONNX packs containing a detector, recognizer,
character dictionary and `manifest.json`. An arbitrary ONNX or Piper model is
not suitable. This guide installs **eSlav PP-OCRv5 Mobile**, whose manifest
declares Russian and English: `ch_PP-OCRv5_det_mobile` finds text regions and
`eslav_PP-OCRv5_rec_mobile` reads characters with its matching dictionary.

## Get the files

Open **Settings → Models** (if the catalog is not downloaded yet, click **Download catalog from repository**) and under **OCR**, select **Download** next to **eSlav PP-OCRv5 Mobile**. You can also reach this section from the OCR panel
using **Download a model** or **Manage models**. When the package shows
**On disk**, return to OCR and select it in the model picker.

### Install manually

1. Under **Settings → General → Folders**, open **Program data (audio cache and models)** and create
   `models\ocr\eslav-ppocrv5-mobile`. This opens your current data folder,
   including any custom location.
2. Download these files from RapidAI/RapidOCR on ModelScope:

| Save as | Source |
|---|---|
| `det.onnx` | [ch_PP-OCRv5_det_mobile.onnx](https://www.modelscope.cn/models/RapidAI/RapidOCR/resolve/master/onnx/PP-OCRv5/det/ch_PP-OCRv5_det_mobile.onnx) |
| `rec.onnx` | [eslav_PP-OCRv5_rec_mobile.onnx](https://www.modelscope.cn/models/RapidAI/RapidOCR/resolve/master/onnx/PP-OCRv5/rec/eslav_PP-OCRv5_rec_mobile.onnx) |
| `dict.txt` | [ppocrv5_eslav_dict.txt](https://www.modelscope.cn/models/RapidAI/RapidOCR/resolve/master/paddle/PP-OCRv5/rec/eslav_PP-OCRv5_rec_mobile/ppocrv5_eslav_dict.txt) |

3. Create **manifest.json** beside them:

```json
{
  "id": "eslav-ppocrv5-mobile",
  "display_name": "eSlav PP-OCRv5 Mobile",
  "languages": ["ru", "en"],
  "family": "pp_ocr_v5",
  "det_file": "det.onnx",
  "rec_file": "rec.onnx",
  "dict_file": "dict.txt"
}
```

Save UTF-8 JSON. In Notepad choose All files to avoid `manifest.json.txt`.
Download the dictionary as a file and keep its line order unchanged.
The original guide checked these links on 4 September 2026. They reference
`master`, so upstream content may change. Retain a matching set rather than
mixing recognizer/dictionary versions.

## Check placement

```text
%LOCALAPPDATA%\ttsbard\models\ocr\
  eslav-ppocrv5-mobile\
    manifest.json
    det.onnx
    rec.onnx
    dict.txt
```

Only immediate `ocr` subfolders are scanned. The folder name must equal the
manifest `id`; files directly under `ocr` or in a second wrapper directory
are not discovered. Each additional pack needs its own folder/unique ID:
lowercase Latin letters, digits and single hyphens, none at either end.
The three file fields must contain different filenames in that same folder,
without subpaths. Discovery does not prove ONNX compatibility; loading does.

## Model search locations

1. `<process working directory>\models\ocr\`, determined by launch method
   (usually the shortcut's Start in field). TTSBard does not write here.
2. `%LOCALAPPDATA%\ttsbard\models\ocr\` or your custom application data folder.

Both roots appear in the list. Duplicate IDs prefer the working directory,
but an invalid working-directory pack does not hide a valid data-directory
pack. Open folder opens Program data. Models beside the executable are
found only if that is also the working directory.

## Enable and test

1. Open **OCR** in the integrations section of the sidebar.
2. Click **Refresh the model list** and choose **eSlav PP-OCRv5 Mobile**.
3. Turn on **Enable OCR** and wait for **Loaded**. Read any panel error;
   a checked box alone is insufficient.
4. Under **Screen**, select All screens, Primary screen or a specific monitor.
   The default selection is **All screens**. Opening the panel refreshes the list; use its
   refresh button after display changes.
5. Press Ctrl+Shift+P and select several text lines on the frozen snapshot.
   Later screen changes do not alter captured text. Escape or clicking another
   application cancels selection.
6. Check **Incoming**. Automatic acceptance queues speech; manual mode waits
   for an action.

Change capture's shortcut under **Hotkeys**. Enabled OCR also starts next launch.
This is one-shot capture, not continuous screen watching. See
[manual review](incoming.md#manual-review).

## Screen selection

On an extended desktop the snapshot window covers the chosen monitor only.
Other monitors keep showing applications. Moving the cursor elsewhere does
not cancel selection, but clicking another application does by losing focus.
Primary screen is resolved on every capture. A specific-monitor choice persists;
if disconnected it stays unavailable instead of falling back. Reconnect/refresh
or choose another screen. Port/display configuration changes may require
reselection. A changed choice applies to the next capture. Mirrored displays
show the same desktop, including the OCR window, on both physical devices.

## If the model does not work

| Symptom | Check |
|---|---|
| No models found | Search roots, one subfolder level, exact manifest filename and folder/ID match; refresh |
| Pack missing | All four files, valid JSON and `family: pp_ocr_v5`; invalid manifests are skipped |
| Listed but loading fails | Download a complete matching set; make sure ONNX files are not HTML error pages |
| Old model after replacement | Disable OCR before replacing files, refresh and re-enable |
| Capture does not start | Wait for readiness, check binding and elevated-app [FAQ](../faq.md) |
| Poor recognition | Select tightly, use readable text and avoid transition animations; check pack language |
| Text appears but no sound | Check automatic acceptance, queue, TTS provider and output; these follow OCR |
