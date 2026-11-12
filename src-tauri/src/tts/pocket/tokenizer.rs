//! Text preparation and tokenization for Pocket TTS packages.
//!
//! Two on-disk formats are supported:
//!
//! * HuggingFace `tokenizer.json` (the English Kyutai package) is loaded with
//!   the same `tokenizers` crate the RUAccent runtime already uses.
//! * SentencePiece `.model` (the Russian Shtorm package) is a self-contained
//!   protobuf. Only the vocabulary, scores and normalizer flags are needed, so
//!   the model is parsed here without adding a C++ dependency to the installer.

use std::collections::HashMap;
use std::path::Path;

use unicode_normalization::UnicodeNormalization;

use super::PocketError;

/// Maximum number of tokens handed to the flow model in one chunk.
pub const MAX_TOKENS_PER_CHUNK: usize = 50;

/// The number of frames generated after EOS. The exported graphs and the
/// reference smoke-cycle use three frames; keeping this constant avoids a
/// dependency on the Python-side `frame_after_eos` heuristic.
pub const FRAMES_AFTER_EOS: u32 = 3;

/// Minimum frames before an EOS prediction is honored.
pub const MIN_FRAMES_BEFORE_EOS: u32 = 6;

#[derive(Debug)]
enum PocketTokenizerImpl {
    Json(Box<tokenizers::Tokenizer>),
    SentencePiece(SentencePieceBpe),
}

/// Tokenizer selected by the package descriptor.
#[derive(Debug)]
pub struct PocketTokenizer {
    inner: PocketTokenizerImpl,
    vocab_size: usize,
}

impl PocketTokenizer {
    pub fn json(path: &Path, expected_vocab: usize) -> Result<Self, PocketError> {
        let tokenizer = tokenizers::Tokenizer::from_file(path).map_err(|error| {
            PocketError::Tokenizer(format!(
                "failed to load tokenizer.json '{}': {error}",
                path.display()
            ))
        })?;
        let vocab_size = tokenizer.get_vocab_size(false);
        if expected_vocab != 0 && vocab_size != expected_vocab {
            return Err(PocketError::Tokenizer(format!(
                "tokenizer.json vocab={vocab_size}, config n_bins={expected_vocab}"
            )));
        }
        Ok(Self {
            inner: PocketTokenizerImpl::Json(Box::new(tokenizer)),
            vocab_size,
        })
    }

    pub fn sentencepiece(path: &Path, expected_vocab: usize) -> Result<Self, PocketError> {
        let bytes = std::fs::read(path).map_err(|error| {
            PocketError::Tokenizer(format!(
                "failed to read tokenizer.model '{}': {error}",
                path.display()
            ))
        })?;
        let bpe = SentencePieceBpe::parse(&bytes)?;
        if expected_vocab != 0 && bpe.vocab_size != expected_vocab {
            return Err(PocketError::Tokenizer(format!(
                "tokenizer.model vocab={}, config n_bins={expected_vocab}",
                bpe.vocab_size
            )));
        }
        let vocab_size = bpe.vocab_size;
        Ok(Self {
            inner: PocketTokenizerImpl::SentencePiece(bpe),
            vocab_size,
        })
    }

    /// Validate a tokenizer file without retaining it. Used by discovery so a
    /// corrupt package is rejected before it appears in the UI.
    pub fn validate(
        path: &Path,
        kind: super::PocketTokenizerKind,
        expected_vocab: usize,
    ) -> Result<(), PocketError> {
        match kind {
            super::PocketTokenizerKind::Json => Self::json(path, expected_vocab).map(|_| ()),
            super::PocketTokenizerKind::SentencePiece => {
                Self::sentencepiece(path, expected_vocab).map(|_| ())
            }
        }
    }

    pub fn vocab_size(&self) -> usize {
        self.vocab_size
    }

    pub fn encode(&self, text: &str) -> Result<Vec<i64>, PocketError> {
        if text.is_empty() {
            return Ok(Vec::new());
        }
        match &self.inner {
            PocketTokenizerImpl::Json(tokenizer) => tokenizer
                .encode(text, false)
                .map(|encoding| encoding.get_ids().iter().map(|id| *id as i64).collect())
                .map_err(|error| PocketError::Tokenizer(error.to_string())),
            PocketTokenizerImpl::SentencePiece(bpe) => bpe.encode(text),
        }
    }
}

/// A single SentencePiece vocabulary entry.
#[derive(Debug, Clone)]
struct SentencePiece {
    id: usize,
    piece: String,
    score: f32,
}

/// Minimal SentencePiece BPE model.
#[derive(Debug)]
struct SentencePieceBpe {
    pieces: HashMap<String, SentencePiece>,
    /// Pieces ordered by id; retained for diagnostics and validation.
    vocab_size: usize,
    unknown_id: usize,
    add_dummy_prefix: bool,
    remove_extra_whitespaces: bool,
    escape_whitespaces: bool,
}

impl SentencePieceBpe {
    fn parse(bytes: &[u8]) -> Result<Self, PocketError> {
        let mut cursor = ProtoCursor::new(bytes);
        let mut pieces = Vec::new();
        let mut add_dummy_prefix = true;
        let mut remove_extra_whitespaces = true;
        let mut escape_whitespaces = true;

        while !cursor.is_empty() {
            let tag = cursor.varint()?;
            let field = tag >> 3;
            let wire = tag & 0x7;
            match (field, wire) {
                (1, 2) => {
                    let message = cursor.length_delimited()?;
                    pieces.push(parse_piece(message)?);
                }
                (3, 2) => {
                    let message = cursor.length_delimited()?;
                    parse_normalizer_spec(
                        message,
                        &mut add_dummy_prefix,
                        &mut remove_extra_whitespaces,
                        &mut escape_whitespaces,
                    )?;
                }
                (_, 0) => {
                    cursor.varint()?;
                }
                (_, 1) => cursor.skip(8)?,
                (_, 2) => {
                    let length = cursor.varint()? as usize;
                    cursor.skip(length)?;
                }
                (_, 5) => cursor.skip(4)?,
                _ => {
                    return Err(PocketError::Tokenizer(format!(
                        "unsupported SentencePiece protobuf wire type {wire}"
                    )));
                }
            }
        }

        if pieces.is_empty() {
            return Err(PocketError::Tokenizer(
                "SentencePiece model has no pieces".to_string(),
            ));
        }
        let mut map: HashMap<String, SentencePiece> = HashMap::with_capacity(pieces.len());
        let mut unknown_id = 0usize;
        for (index, mut piece) in pieces.into_iter().enumerate() {
            if let Some(existing) = map.get(&piece.piece) {
                return Err(PocketError::Tokenizer(format!(
                    "duplicate SentencePiece piece '{}' (ids {} and {})",
                    piece.piece, existing.id, index
                )));
            }
            piece.id = index;
            if piece.piece == "<unk>" {
                unknown_id = index;
            }
            map.insert(piece.piece.clone(), piece);
        }
        let vocab_size = map.len();
        Ok(Self {
            pieces: map,
            vocab_size,
            unknown_id,
            add_dummy_prefix,
            remove_extra_whitespaces,
            escape_whitespaces,
        })
    }

    fn encode(&self, text: &str) -> Result<Vec<i64>, PocketError> {
        let normalized = self.normalize(text);
        if normalized.is_empty() {
            return Ok(Vec::new());
        }

        let mut escaped = normalized;
        if self.escape_whitespaces {
            escaped = escaped.replace(' ', "\u{2581}");
            if self.add_dummy_prefix && !escaped.starts_with('\u{2581}') {
                escaped.insert(0, '\u{2581}');
            }
        } else if self.add_dummy_prefix && !escaped.starts_with(' ') {
            escaped.insert(0, ' ');
        }

        let mut symbols: Vec<String> = escaped.chars().map(|ch| ch.to_string()).collect();
        loop {
            let mut best: Option<(usize, f32)> = None;
            for index in 0..symbols.len().saturating_sub(1) {
                let mut pair =
                    String::with_capacity(symbols[index].len() + symbols[index + 1].len());
                pair.push_str(&symbols[index]);
                pair.push_str(&symbols[index + 1]);
                let Some(piece) = self.pieces.get(&pair) else {
                    continue;
                };
                if best.is_none_or(|(_, best_score)| piece.score > best_score) {
                    best = Some((index, piece.score));
                }
            }
            let Some((index, _)) = best else { break };
            let mut merged = String::with_capacity(symbols[index].len() + symbols[index + 1].len());
            merged.push_str(&symbols[index]);
            merged.push_str(&symbols[index + 1]);
            symbols.splice(index..index + 2, [merged]);
        }

        Ok(symbols
            .iter()
            .map(|symbol| {
                self.pieces
                    .get(symbol)
                    .map(|piece| piece.id as i64)
                    .unwrap_or(self.unknown_id as i64)
            })
            .collect())
    }

    fn normalize(&self, text: &str) -> String {
        let nfkc: String = text.nfkc().collect();
        let mut spaced = String::with_capacity(nfkc.len());
        for ch in nfkc.chars() {
            if ch.is_control() {
                if ch.is_whitespace() {
                    spaced.push(' ');
                }
            } else if ch.is_whitespace() {
                spaced.push(' ');
            } else {
                spaced.push(ch);
            }
        }
        if self.remove_extra_whitespaces {
            spaced.split_whitespace().collect::<Vec<_>>().join(" ")
        } else {
            spaced
        }
    }
}

fn parse_piece(message: &[u8]) -> Result<SentencePiece, PocketError> {
    let mut cursor = ProtoCursor::new(message);
    let mut piece = None;
    let mut score = 0.0f32;
    while !cursor.is_empty() {
        let tag = cursor.varint()?;
        let field = tag >> 3;
        let wire = tag & 0x7;
        match (field, wire) {
            (1, 2) => {
                let raw = cursor.length_delimited()?;
                piece = Some(
                    std::str::from_utf8(raw)
                        .map_err(|_| {
                            PocketError::Tokenizer(
                                "SentencePiece piece is not valid UTF-8".to_string(),
                            )
                        })?
                        .to_string(),
                );
            }
            (2, 5) => {
                let raw = cursor.take(4)?;
                score = f32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]);
            }
            (_, 0) => {
                cursor.varint()?;
            }
            (_, 1) => cursor.skip(8)?,
            (_, 2) => {
                let length = cursor.varint()? as usize;
                cursor.skip(length)?;
            }
            (_, 5) => cursor.skip(4)?,
            _ => {
                return Err(PocketError::Tokenizer(
                    "unsupported SentencePiece piece protobuf".to_string(),
                ));
            }
        }
    }
    let piece = piece.ok_or_else(|| {
        PocketError::Tokenizer("SentencePiece piece is missing its text".to_string())
    })?;
    Ok(SentencePiece {
        id: 0,
        piece,
        score,
    })
}

fn parse_normalizer_spec(
    message: &[u8],
    add_dummy_prefix: &mut bool,
    remove_extra_whitespaces: &mut bool,
    escape_whitespaces: &mut bool,
) -> Result<(), PocketError> {
    let mut cursor = ProtoCursor::new(message);
    while !cursor.is_empty() {
        let tag = cursor.varint()?;
        let field = tag >> 3;
        let wire = tag & 0x7;
        match (field, wire) {
            (3, 0) => *add_dummy_prefix = cursor.varint()? != 0,
            (4, 0) => *remove_extra_whitespaces = cursor.varint()? != 0,
            (5, 0) => *escape_whitespaces = cursor.varint()? != 0,
            (_, 0) => {
                cursor.varint()?;
            }
            (_, 1) => cursor.skip(8)?,
            (_, 2) => {
                let length = cursor.varint()? as usize;
                cursor.skip(length)?;
            }
            (_, 5) => cursor.skip(4)?,
            _ => {
                return Err(PocketError::Tokenizer(
                    "unsupported SentencePiece normalizer protobuf".to_string(),
                ));
            }
        }
    }
    Ok(())
}

struct ProtoCursor<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> ProtoCursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn is_empty(&self) -> bool {
        self.offset >= self.bytes.len()
    }

    fn take(&mut self, count: usize) -> Result<&'a [u8], PocketError> {
        let end = self.offset.checked_add(count).ok_or_else(|| {
            PocketError::Tokenizer("SentencePiece protobuf length overflow".to_string())
        })?;
        if end > self.bytes.len() {
            return Err(PocketError::Tokenizer(
                "truncated SentencePiece protobuf".to_string(),
            ));
        }
        let slice = &self.bytes[self.offset..end];
        self.offset = end;
        Ok(slice)
    }

    fn skip(&mut self, count: usize) -> Result<(), PocketError> {
        self.take(count).map(|_| ())
    }

    fn varint(&mut self) -> Result<u64, PocketError> {
        let mut value = 0u64;
        let mut shift = 0u32;
        loop {
            let byte = *self.take(1)?.first().expect("one byte was requested");
            value |= ((byte & 0x7f) as u64) << shift;
            if byte & 0x80 == 0 {
                return Ok(value);
            }
            shift += 7;
            if shift >= 64 {
                return Err(PocketError::Tokenizer(
                    "SentencePiece protobuf varint is too long".to_string(),
                ));
            }
        }
    }

    fn length_delimited(&mut self) -> Result<&'a [u8], PocketError> {
        let length = self.varint()? as usize;
        self.take(length)
    }
}

/// Prepare a phrase for the model, following the reference Python pipeline for
/// the parts that do not depend on a specific tokenizer.
pub fn prepare_text(text: &str, language: &str) -> Result<String, PocketError> {
    let mut text = text.trim().replace(['\r', '\n'], " ");
    while text.contains("  ") {
        text = text.replace("  ", " ");
    }
    if language.eq_ignore_ascii_case("en") {
        text = text.replace(['\u{2019}', '\u{2018}'], "'");
    }
    if text.is_empty() {
        return Err(PocketError::Generation(
            "text prompt cannot be empty".to_string(),
        ));
    }
    if let Some(first) = text.chars().next() {
        if first.is_lowercase() {
            let upper: String = first.to_uppercase().collect();
            text.replace_range(0..first.len_utf8(), &upper);
        }
    }
    text = ensure_terminal_punctuation(&text);
    Ok(text)
}

fn ensure_terminal_punctuation(text: &str) -> String {
    const CLOSERS: &str = "\"'\u{201d}\u{2019})]}\u{00bb}";
    const TERMINAL: &str = ".!?\u{2026}";
    const WEAK: &str = ",;:-\u{2013}\u{2014}";

    let core = text.trim_end_matches(|ch: char| CLOSERS.contains(ch) || ch.is_whitespace());
    let closers = text[core.len()..].trim();
    if core.is_empty() {
        return ".".to_string();
    }
    let last = core.chars().next_back().expect("core is not empty");
    if TERMINAL.contains(last) {
        return text.to_string();
    }
    if WEAK.contains(last) {
        let trimmed = core.trim_end_matches(|ch: char| WEAK.contains(ch) || ch.is_whitespace());
        return format!("{trimmed}.{closers}");
    }
    format!("{text}.")
}

/// Split a prepared text into chunks accepted by the flow model.
pub fn split_text_chunks(
    tokenizer: &PocketTokenizer,
    language: &str,
    text: &str,
    max_tokens: usize,
) -> Result<Vec<String>, PocketError> {
    let prepared = prepare_text(text, language)?;
    if tokenizer.encode(&prepared)?.len() <= max_tokens {
        return Ok(vec![prepared]);
    }

    let sentences = split_sentences(&prepared);
    let mut chunks: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut current_tokens = 0usize;

    for sentence in sentences {
        let sentence = sentence.trim();
        if sentence.is_empty() {
            continue;
        }
        let sentence_tokens = tokenizer.encode(sentence)?.len();
        if sentence_tokens > max_tokens {
            if !current.is_empty() {
                chunks.push(current.trim().to_string());
                current.clear();
                current_tokens = 0;
            }
            chunks.extend(split_long_sentence(tokenizer, sentence, max_tokens)?);
            continue;
        }
        if current.is_empty() {
            current = sentence.to_string();
            current_tokens = sentence_tokens;
        } else if current_tokens + sentence_tokens <= max_tokens {
            current.push(' ');
            current.push_str(sentence);
            current_tokens += sentence_tokens;
        } else {
            chunks.push(current.trim().to_string());
            current = sentence.to_string();
            current_tokens = sentence_tokens;
        }
    }
    if !current.trim().is_empty() {
        chunks.push(current.trim().to_string());
    }
    if chunks.is_empty() {
        chunks.push(prepared);
    }
    for chunk in &mut chunks {
        *chunk = ensure_terminal_punctuation(chunk.trim());
    }
    Ok(chunks)
}

fn split_sentences(text: &str) -> Vec<String> {
    const TERMINAL: &str = ".!?\u{2026}";
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut result = Vec::new();
    let mut start = 0usize;
    let mut index = 0usize;
    while index < chars.len() {
        let (_byte, ch) = chars[index];
        if TERMINAL.contains(ch) {
            let previous = if index > 0 {
                Some(chars[index - 1].1)
            } else {
                None
            };
            let mut next_index = index + 1;
            while next_index < chars.len()
                && " \t\"'\u{201d}\u{2019})]}\u{00bb}".contains(chars[next_index].1)
            {
                next_index += 1;
            }
            let next = chars.get(next_index).map(|(_, ch)| *ch);
            let decimal = previous.is_some_and(|ch| ch.is_ascii_digit())
                && next.is_some_and(|ch| ch.is_ascii_digit());
            if !decimal {
                let end = chars
                    .get(next_index)
                    .map(|(byte, _)| *byte)
                    .unwrap_or(text.len());
                result.push(text[start..end].to_string());
                start = end;
                index = next_index;
                continue;
            }
        }
        index += 1;
    }
    if start < text.len() {
        result.push(text[start..].to_string());
    }
    result
}

fn split_long_sentence(
    tokenizer: &PocketTokenizer,
    sentence: &str,
    max_tokens: usize,
) -> Result<Vec<String>, PocketError> {
    let words: Vec<&str> = sentence.split_whitespace().collect();
    let mut chunks = Vec::new();
    let mut current = String::new();
    for word in words {
        let candidate = if current.is_empty() {
            word.to_string()
        } else {
            format!("{current} {word}")
        };
        if tokenizer.encode(&candidate)?.len() > max_tokens && !current.is_empty() {
            chunks.push(current);
            current = word.to_string();
        } else {
            current = candidate;
        }
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    Ok(chunks)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bpe(entries: &[(&str, f32)]) -> SentencePieceBpe {
        let pieces = entries
            .iter()
            .enumerate()
            .map(|(id, (piece, score))| {
                (
                    (*piece).to_string(),
                    SentencePiece {
                        id,
                        piece: (*piece).to_string(),
                        score: *score,
                    },
                )
            })
            .collect();
        SentencePieceBpe {
            pieces,
            vocab_size: entries.len(),
            unknown_id: 0,
            add_dummy_prefix: true,
            remove_extra_whitespaces: true,
            escape_whitespaces: true,
        }
    }

    #[test]
    fn sentencepiece_bpe_prefers_highest_merge_score() {
        let tokenizer = bpe(&[
            ("<unk>", 0.0),
            ("\u{2581}", -10.0),
            ("a", -9.0),
            ("b", -8.0),
            ("\u{2581}a", -1.0),
            ("ab", -2.0),
            ("\u{2581}ab", 0.0),
        ]);
        let ids = tokenizer.encode(" ab").unwrap();
        assert_eq!(ids, vec![6]);
    }

    #[test]
    fn sentencepiece_bpe_maps_unknown_to_unk() {
        let tokenizer = bpe(&[("<unk>", 0.0), ("\u{2581}", -1.0), ("a", -2.0)]);
        assert_eq!(tokenizer.encode("z").unwrap(), vec![1, 0]);
    }

    #[test]
    fn sentencepiece_normalizes_whitespace_and_keeps_stress() {
        let tokenizer = bpe(&[
            ("<unk>", 0.0),
            ("\u{2581}", -10.0),
            ("о", -9.0),
            ("\u{301}", -8.0),
            ("\u{2581}о", -7.0),
            ("\u{2581}о\u{301}", 0.0),
        ]);
        let ids = tokenizer.encode("  о\u{301}  ").unwrap();
        assert_eq!(ids, vec![5]);
    }

    #[test]
    fn prepare_text_capitalizes_and_adds_terminal_punctuation() {
        assert_eq!(
            prepare_text("  hello   world  ", "en").unwrap(),
            "Hello world."
        );
        assert_eq!(prepare_text("привет, мир", "ru").unwrap(), "Привет, мир.");
        assert_eq!(prepare_text("Что?", "ru").unwrap(), "Что?");
    }

    #[test]
    fn prepare_text_replaces_english_typographic_apostrophes() {
        assert_eq!(
            prepare_text("it\u{2019}s fine", "en").unwrap(),
            "It's fine."
        );
    }

    #[test]
    fn ensure_terminal_punctuation_keeps_closers_after_period() {
        assert_eq!(ensure_terminal_punctuation("Привет, мир,"), "Привет, мир.");
        assert_eq!(
            ensure_terminal_punctuation("\u{00ab}Привет\u{00bb}"),
            "\u{00ab}Привет\u{00bb}."
        );
    }

    #[test]
    fn split_sentences_does_not_split_decimals() {
        assert_eq!(
            split_sentences("Стоимость 3.14 рубля. Далее текст."),
            vec!["Стоимость 3.14 рубля. ", "Далее текст."]
        );
    }

    #[test]
    fn empty_prepared_text_is_rejected() {
        assert!(matches!(
            prepare_text("   ", "ru"),
            Err(PocketError::Generation(_))
        ));
    }
}
