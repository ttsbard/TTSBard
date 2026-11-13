**English | [Русский](../../ru/user/ruaccent.md)**

# RUAccent: Russian stress and homographs

RUAccent is a local model for **Russian text**. It places stress marks and uses
context to distinguish homographs, such as `за́мок` (castle) and `замо́к` (lock).
Text is processed on your computer without uploading it for this step.
RUAccent does not check spelling, translate text or provide pronunciation
processing for English. It is separate from the editor's Russian spell checker;
changing the interface language does not change the model's language.
It supports Silero and Piper. Automatic processing changes only text sent to
synthesis, leaving the editor unchanged.

## Download and prepare a model

Open **Settings → Models** (if the catalog is not downloaded yet, click **Download catalog from repository**), find **RUAccent Tiny (Compact INT8 + FST)** under **Stress placement (RUAccent)**, and select **Download**. When the package shows **On disk**,
continue to the stress placement settings further down the same tab.

### Install manually

The author distributes weights at
[ruaccent/accentuator](https://huggingface.co/ruaccent/accentuator).
Under **Settings → General → Folders**, open **Program data (audio cache and models)** and create
`models\ruaccent`:

```text
%LOCALAPPDATA%\ttsbard\models\ruaccent\
```

Use the same relative path under a custom data directory. Alternatively use
`<process working directory>\models\ruaccent`, controlled by the shortcut's
“Start in” field. Both roots are scanned; duplicate variants prefer the working
directory, which TTSBard does not modify.

### Manual download

The compact catalog package ships `dictionary/accents.fst`. The runtime also
accepts `dictionary/accents.json.gz` from the upstream files shown below.

1. Open [Files](https://huggingface.co/ruaccent/accentuator/tree/main).
2. Download the shared `dictionary` and `nn` files shown below, preserving paths.
3. Under [nn/nn_omograph](https://huggingface.co/ruaccent/accentuator/tree/main/nn/nn_omograph),
   select **one** variant, for example `tiny`, and download its files into that
   variant's folder. You do not need all variants.

```text
%LOCALAPPDATA%\ttsbard\models\ruaccent\
  dictionary/
    accents.json.gz
    omographs.json.gz
    yo_words.json.gz
    yo_homographs.json.gz
  nn/
    nn_accent/
      model.onnx
      config.json
      vocab.txt
    nn_omograph/
      tiny/
        model.onnx
        tokenizer.json
    nn_stress_usage_predictor/
      model.onnx
      config.json
      tokenizer.json
    nn_yo_homograph_resolver/
      model.onnx
      config.json
      tokenizer.json
```

Place `nn` and `dictionary` directly under `ruaccent`, without another wrapper
folder. Complete variants are discovered under `nn/nn_omograph`. Restart TTSBard.

### Optional Python download

The RUAccent library's downloader can fetch common files and `tiny`:

```powershell
python -m pip install ruaccent
python -c "from ruaccent import RUAccent; RUAccent().load(omograph_model_size='tiny', use_dictionary=True, workdir=r'$env:LOCALAPPDATA\ttsbard\models\ruaccent')"
```

Python is only needed for this alternative download method. TTSBard itself
uses a built-in native runtime.

## Enable automatic processing

1. Open **Settings → Models → Stress placement**.
2. Select an installed RUAccent model.
3. Click the load button beside the list and wait for **Ready**.
4. Enable **Place stresses automatically** to process each message and resolve
   homographs before synthesis.

Models do not load automatically at synthesis time by default. Enable **Load
on startup** to avoid manual loading each launch; startup may take longer.
On failure, check the files and use the retry button. If a model is missing,
verify folder nesting and `model.onnx`/`tokenizer.json` in the selected variant.

## Process just one phrase

Leave automatic processing off and choose **Place stresses** from the editor's
**…** menu or press `Ctrl+U` (configurable under Hotkeys → Editor). It changes
current editor text so you can review it before sending. A selected model must
be **Ready**, but automatic processing need not be enabled.

## License and updates

The author distributes RUAccent code/model under MIT; consult the model page
for its terms and current files.
