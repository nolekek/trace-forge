//! # Semantic Loop & Thrashing Detector
//!
//! Sub-millisecond vector engine and sliding window analyzer for autonomous LLM agents.
//! Detects agent thrashing, repetitive ineffective tool calls, and infinite loops
//! using feature hashing / token n-gram vectorization with L2 normalization
//! and cosine similarity.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::Arc;
use std::time::SystemTime;

/// Synthetic recovery prompt injected to steer agents away from repetitive action loops.
pub const RECOVERY_PROMPT: &str =
    "CircuitBreaker: Action loop detected with high semantic similarity. You are repeating ineffective tool calls. Stop and change strategy.";

/// Default sliding window size for action history.
pub const DEFAULT_WINDOW_SIZE: usize = 5;

/// Default cosine similarity threshold triggering thrashing alerts (92%).
pub const DEFAULT_SIMILARITY_THRESHOLD: f32 = 0.92;

/// Default threshold of consecutive similar actions before circuit breaker fires.
pub const DEFAULT_CONSECUTIVE_REPEATS_THRESHOLD: usize = 3;

/// Default embedding vector dimension (matches all-MiniLM-L6-v2).
pub const DEFAULT_EMBEDDING_DIMENSION: usize = 384;

// ---------------------------------------------------------------------------
// ActionRecord & LoopDetectionResult
// ---------------------------------------------------------------------------

/// Historical record of an agent action within the sliding window.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionRecord {
    /// Normalized tool name (e.g. "execute_bash", "cat", "read_file").
    pub tool_name: String,
    /// Normalized tool arguments.
    pub tool_args: serde_json::Value,
    /// Optional agent reasoning or thought content.
    pub thought: Option<String>,
    /// L2-normalized semantic embedding vector.
    pub embedding: Vec<f32>,
    /// Timestamp in milliseconds since UNIX epoch.
    pub timestamp_ms: u64,
}

/// Result of evaluating an action for semantic looping / thrashing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LoopDetectionResult {
    /// Whether an action loop / thrashing pattern was detected.
    pub is_loop: bool,
    /// Cosine similarity score between current and preceding action (or 0.0 if first action).
    pub similarity_score: f32,
    /// Current count of consecutive actions exceeding the similarity threshold.
    pub repeated_count: usize,
    /// Corrective prompt instructing the agent to break out of the loop.
    pub recovery_prompt: Option<String>,
}

// ---------------------------------------------------------------------------
// Vectorizer Trait & Implementations
// ---------------------------------------------------------------------------

/// Abstraction for converting action descriptions into float embeddings.
pub trait ActionVectorizer: Send + Sync {
    /// Vectorize action text into an L2-normalized float embedding.
    fn vectorize(&self, text: &str) -> Vec<f32>;

    /// Return the embedding dimensionality.
    fn dimension(&self) -> usize;

    /// Return the descriptive name of the vectorizer backend.
    fn name(&self) -> &str;
}

/// Fast, lock-free, zero-dependency feature hashing vectorizer.
///
/// Uses word n-grams (unigrams, bigrams) and character n-grams (3-grams, 4-grams)
/// with 64-bit FNV-1a signed feature hashing (Weinberger et al.) and L2 normalization.
/// Average latency is < 5 microseconds per vectorization.
#[derive(Debug, Clone)]
pub struct FeatureHashingVectorizer {
    dimension: usize,
    char_ngram_min: usize,
    char_ngram_max: usize,
    include_word_unigrams: bool,
    include_word_bigrams: bool,
}

impl Default for FeatureHashingVectorizer {
    fn default() -> Self {
        Self::with_dimension(DEFAULT_EMBEDDING_DIMENSION)
    }
}

impl FeatureHashingVectorizer {
    /// Create a new feature hashing vectorizer with the specified dimension.
    pub fn with_dimension(dimension: usize) -> Self {
        Self {
            dimension: dimension.max(16),
            char_ngram_min: 3,
            char_ngram_max: 4,
            include_word_unigrams: true,
            include_word_bigrams: true,
        }
    }
}

/// 64-bit FNV-1a deterministic hash.
#[inline]
fn fnv1a_hash(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

impl ActionVectorizer for FeatureHashingVectorizer {
    fn vectorize(&self, text: &str) -> Vec<f32> {
        let mut vec = vec![0.0f32; self.dimension];

        // 1. Word tokens (unigrams and bigrams)
        let words: Vec<&str> = text
            .split(|c: char| {
                c.is_whitespace()
                    || c == ','
                    || c == ';'
                    || c == ':'
                    || c == '"'
                    || c == '\''
                    || c == '{'
                    || c == '}'
                    || c == '['
                    || c == ']'
            })
            .filter(|s| !s.is_empty())
            .collect();

        if self.include_word_unigrams {
            for &w in &words {
                let lower = w.to_lowercase();
                let mut key = String::with_capacity(2 + lower.len());
                key.push_str("w:");
                key.push_str(&lower);

                let h = fnv1a_hash(key.as_bytes());
                let idx = (h as usize) % self.dimension;
                let sign = if ((h >> 32) & 1) == 0 { 1.0f32 } else { -1.0f32 };
                vec[idx] += sign * 1.5;
            }
        }

        if self.include_word_bigrams && words.len() >= 2 {
            for i in 0..words.len() - 1 {
                let w1 = words[i].to_lowercase();
                let w2 = words[i + 1].to_lowercase();
                let mut key = String::with_capacity(3 + w1.len() + w2.len());
                key.push_str("b:");
                key.push_str(&w1);
                key.push('_');
                key.push_str(&w2);

                let h = fnv1a_hash(key.as_bytes());
                let idx = (h as usize) % self.dimension;
                let sign = if ((h >> 32) & 1) == 0 { 1.0f32 } else { -1.0f32 };
                vec[idx] += sign * 2.0;
            }
        }

        // 2. Character n-grams (3-grams and 4-grams)
        let lower_text = text.to_lowercase();
        let chars: Vec<char> = lower_text
            .chars()
            .filter(|c| !c.is_control())
            .collect();

        for n in self.char_ngram_min..=self.char_ngram_max {
            if chars.len() >= n {
                for window in chars.windows(n) {
                    let s: String = window.iter().collect();
                    let mut key = String::with_capacity(2 + s.len());
                    key.push_str("c:");
                    key.push_str(&s);

                    let h = fnv1a_hash(key.as_bytes());
                    let idx = (h as usize) % self.dimension;
                    let sign = if ((h >> 32) & 1) == 0 { 1.0f32 } else { -1.0f32 };
                    vec[idx] += sign * 0.8;
                }
            }
        }

        // 3. L2 Normalization: v = v / ||v||
        let norm_sq: f32 = vec.iter().map(|&x| x * x).sum();
        if norm_sq > 1e-9 {
            let inv_norm = 1.0 / norm_sq.sqrt();
            for x in vec.iter_mut() {
                *x *= inv_norm;
            }
        }

        vec
    }

    fn dimension(&self) -> usize {
        self.dimension
    }

    fn name(&self) -> &str {
        "feature_hashing_fnv1a"
    }
}

/// Adapter interface for ONNX runtime model embeddings (e.g. all-MiniLM-L6-v2).
///
/// Enables seamless substitution with ONNX inference runtime or external inference backends,
/// falling back to fast feature hashing if ONNX is disabled or unavailable.
pub struct OnnxVectorizerAdapter {
    model_name: String,
    dimension: usize,
    inference_fn: Option<Box<dyn Fn(&str) -> Result<Vec<f32>, String> + Send + Sync>>,
    fallback: FeatureHashingVectorizer,
}

impl OnnxVectorizerAdapter {
    /// Create an ONNX vectorizer adapter with default fallback.
    pub fn new(model_name: impl Into<String>, dimension: usize) -> Self {
        Self {
            model_name: model_name.into(),
            dimension,
            inference_fn: None,
            fallback: FeatureHashingVectorizer::with_dimension(dimension),
        }
    }

    /// Create an ONNX vectorizer adapter with an active inference closure.
    pub fn with_inference_fn<F>(
        model_name: impl Into<String>,
        dimension: usize,
        inference_fn: F,
    ) -> Self
    where
        F: Fn(&str) -> Result<Vec<f32>, String> + Send + Sync + 'static,
    {
        Self {
            model_name: model_name.into(),
            dimension,
            inference_fn: Some(Box::new(inference_fn)),
            fallback: FeatureHashingVectorizer::with_dimension(dimension),
        }
    }
}

impl ActionVectorizer for OnnxVectorizerAdapter {
    fn vectorize(&self, text: &str) -> Vec<f32> {
        if let Some(ref f) = self.inference_fn {
            match f(text) {
                Ok(vec) if vec.len() == self.dimension => return vec,
                Ok(_) => {
                    tracing::warn!(
                        "ONNX embedding dimension mismatch, falling back to feature hashing"
                    );
                }
                Err(err) => {
                    tracing::warn!("ONNX inference error: {err}, falling back to feature hashing");
                }
            }
        }
        self.fallback.vectorize(text)
    }

    fn dimension(&self) -> usize {
        self.dimension
    }

    fn name(&self) -> &str {
        &self.model_name
    }
}

// ---------------------------------------------------------------------------
// Normalization & Similarity
// ---------------------------------------------------------------------------

/// Compute the Cosine Similarity between two embedding vectors:
/// `dot_product / (norm_a * norm_b)` clamped to `[-1.0, 1.0]`.
#[inline]
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.is_empty() || b.is_empty() || a.len() != b.len() {
        return 0.0;
    }

    let mut dot = 0.0f32;
    let mut norm_a_sq = 0.0f32;
    let mut norm_b_sq = 0.0f32;

    for (&x, &y) in a.iter().zip(b.iter()) {
        dot += x * y;
        norm_a_sq += x * x;
        norm_b_sq += y * y;
    }

    let denom = norm_a_sq.sqrt() * norm_b_sq.sqrt();
    if denom < 1e-9 {
        0.0
    } else {
        (dot / denom).clamp(-1.0, 1.0)
    }
}

/// Normalize whitespace and path prefixes in a string.
pub fn normalize_string(s: &str) -> String {
    // 1. Collapse multiple whitespace characters into a single space
    let mut collapsed = String::with_capacity(s.len());
    let mut prev_is_space = false;
    for c in s.chars() {
        if c.is_whitespace() {
            if !prev_is_space {
                collapsed.push(' ');
                prev_is_space = true;
            }
        } else {
            collapsed.push(c);
            prev_is_space = false;
        }
    }
    let trimmed = collapsed.trim();

    // 2. Normalize path prefixes: remove redundant `./` (e.g., `cat ./file.txt` -> `cat file.txt`)
    normalize_path_prefixes(trimmed)
}

/// Remove redundant `./` path prefixes while preserving `../`.
fn normalize_path_prefixes(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let chars: Vec<char> = s.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        // Look for `./`
        if chars[i] == '.' && i + 1 < len && chars[i + 1] == '/' {
            let is_boundary = i == 0
                || chars[i - 1].is_whitespace()
                || chars[i - 1] == '"'
                || chars[i - 1] == '\''
                || chars[i - 1] == '/'
                || chars[i - 1] == '\\';

            let is_not_parent_dir = i == 0 || chars[i - 1] != '.';

            if is_boundary && is_not_parent_dir {
                // Skip `./`
                i += 2;
                continue;
            }
        }
        result.push(chars[i]);
        i += 1;
    }

    result
}

/// Deeply normalize a JSON Value:
/// - Strings: whitespace collapsed, path prefixes normalized.
/// - Objects: keys sorted deterministically via BTreeMap, values normalized.
/// - Arrays: all elements recursively normalized.
pub fn normalize_json_value(val: &serde_json::Value) -> serde_json::Value {
    match val {
        serde_json::Value::String(s) => serde_json::Value::String(normalize_string(s)),
        serde_json::Value::Array(arr) => {
            let normalized_arr: Vec<serde_json::Value> =
                arr.iter().map(normalize_json_value).collect();
            serde_json::Value::Array(normalized_arr)
        }
        serde_json::Value::Object(map) => {
            let mut sorted = BTreeMap::new();
            for (k, v) in map {
                sorted.insert(k.trim().to_string(), normalize_json_value(v));
            }
            serde_json::to_value(sorted).unwrap_or_else(|_| val.clone())
        }
        other => other.clone(),
    }
}

/// Construct a canonical formatted text string from tool name, args, and optional thought.
pub fn format_action_for_embedding(
    tool_name: &str,
    normalized_args: &serde_json::Value,
    thought: Option<&str>,
) -> String {
    let mut out = String::with_capacity(128);
    out.push_str("tool:");
    out.push_str(tool_name.trim());
    out.push('\n');
    out.push_str("args:");
    out.push_str(&serde_json::to_string(normalized_args).unwrap_or_default());
    if let Some(t) = thought {
        let trimmed = t.trim();
        if !trimmed.is_empty() {
            out.push_str("\nthought:");
            out.push_str(trimmed);
        }
    }
    out
}

/// Get the current timestamp in milliseconds since UNIX epoch.
fn current_timestamp_ms() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Session State & SemanticLoopDetector
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct SessionState {
    history: VecDeque<ActionRecord>,
    consecutive_repeats: usize,
}

/// Real-Time Semantic Loop and Action Thrashing Detector.
///
/// Tracks sliding window histories of agent tool calls and thoughts per session.
/// Employs fast feature hashing / vector similarity to detect ineffective loops
/// with sub-millisecond execution times.
pub struct SemanticLoopDetector {
    /// Maximum number of action records kept in the sliding history per session.
    pub window_size: usize,
    /// Cosine similarity threshold for declaring two actions semantically identical (0.0 to 1.0).
    pub similarity_threshold: f32,
    /// Number of consecutive high-similarity actions required to trip the circuit breaker.
    pub consecutive_repeats_threshold: usize,
    /// Per-session sliding window history and repeat counters.
    sessions: HashMap<String, SessionState>,
    /// Underlying vectorizer backend.
    vectorizer: Arc<dyn ActionVectorizer>,
}

impl Default for SemanticLoopDetector {
    fn default() -> Self {
        Self::new(
            DEFAULT_WINDOW_SIZE,
            DEFAULT_SIMILARITY_THRESHOLD,
            DEFAULT_CONSECUTIVE_REPEATS_THRESHOLD,
        )
    }
}

impl SemanticLoopDetector {
    /// Create a new `SemanticLoopDetector` with the specified configuration.
    pub fn new(
        window_size: usize,
        similarity_threshold: f32,
        consecutive_repeats_threshold: usize,
    ) -> Self {
        Self {
            window_size: window_size.max(1),
            similarity_threshold,
            consecutive_repeats_threshold: consecutive_repeats_threshold.max(1),
            sessions: HashMap::new(),
            vectorizer: Arc::new(FeatureHashingVectorizer::default()),
        }
    }

    /// Set a custom vectorizer (e.g. an ONNX adapter or custom feature hasher).
    pub fn with_vectorizer(mut self, vectorizer: Arc<dyn ActionVectorizer>) -> Self {
        self.vectorizer = vectorizer;
        self
    }

    /// Check an incoming agent action for semantic looping and thrashing.
    ///
    /// Evaluates the semantic similarity against the previous action in the session.
    /// If similarity >= `similarity_threshold` for `consecutive_repeats_threshold` consecutive steps,
    /// trips the circuit breaker and returns `is_loop: true` with a synthetic recovery prompt.
    pub fn check_action(
        &mut self,
        session_id: &str,
        tool_name: &str,
        args: &serde_json::Value,
        thought: Option<&str>,
    ) -> LoopDetectionResult {
        // 1. Canonicalize action representation
        let normalized_tool = tool_name.trim().to_lowercase();
        let normalized_args = normalize_json_value(args);
        let normalized_thought = thought.map(normalize_string).filter(|s| !s.is_empty());

        let action_text = format_action_for_embedding(
            &normalized_tool,
            &normalized_args,
            normalized_thought.as_deref(),
        );

        // 2. Vectorize action into unit embedding
        let embedding = self.vectorizer.vectorize(&action_text);

        // 3. Retrieve or initialize session state
        let state = self
            .sessions
            .entry(session_id.to_string())
            .or_insert_with(|| SessionState {
                history: VecDeque::with_capacity(self.window_size),
                consecutive_repeats: 0,
            });

        // 4. Compute similarity with the immediately preceding action
        let similarity_score = match state.history.back() {
            Some(prev_action) => {
                // If tool name and normalized args are identical, the action call is 100% identical
                if normalized_tool == prev_action.tool_name
                    && normalized_args == prev_action.tool_args
                {
                    1.0
                } else {
                    cosine_similarity(&embedding, &prev_action.embedding)
                }
            }
            None => 0.0,
        };

        // 5. Update consecutive repeats count
        if state.history.is_empty() {
            state.consecutive_repeats = 1;
        } else if similarity_score >= self.similarity_threshold {
            state.consecutive_repeats += 1;
        } else {
            state.consecutive_repeats = 1;
        }

        // 6. Check if repetition threshold is reached
        let is_loop = state.consecutive_repeats >= self.consecutive_repeats_threshold;
        let recovery_prompt = if is_loop {
            Some(RECOVERY_PROMPT.to_string())
        } else {
            None
        };

        // 7. Store record in bounded sliding window
        let record = ActionRecord {
            tool_name: normalized_tool,
            tool_args: normalized_args,
            thought: normalized_thought,
            embedding,
            timestamp_ms: current_timestamp_ms(),
        };

        state.history.push_back(record);
        if state.history.len() > self.window_size {
            state.history.pop_front();
        }

        LoopDetectionResult {
            is_loop,
            similarity_score,
            repeated_count: state.consecutive_repeats,
            recovery_prompt,
        }
    }

    /// Reset the history and repetition counter for a session.
    pub fn reset_session(&mut self, session_id: &str) -> bool {
        self.sessions.remove(session_id).is_some()
    }

    /// Retrieve the sliding window history for a given session.
    pub fn get_history(&self, session_id: &str) -> Option<&VecDeque<ActionRecord>> {
        self.sessions.get(session_id).map(|s| &s.history)
    }

    /// Retrieve the number of active sessions currently stored.
    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }

    /// Clear all session states.
    pub fn clear(&mut self) {
        self.sessions.clear();
    }
}
