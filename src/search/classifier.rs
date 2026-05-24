//! Query classification for adaptive search strategy selection
//!
//! Analyzes queries to determine their type and complexity, enabling
//! intelligent selection of BM25 vs semantic search weights.
//!
//! ## Query Types
//!
//! - **Symbol**: Bare identifier, namespace-qualified, or contains uppercase
//!   (e.g., `save_pretrained`, `Client`, `foo::bar::baz`)
//! - **SimpleNl**: 1-2 word natural query (e.g., "rust error", "auth flow")
//! - **ComplexNl**: Multi-word natural query, often with relationship words
//!   (e.g., "how does the worker handle payloads")

use regex::Regex;
use std::sync::LazyLock;

/// Relationship words that indicate semantic-heavy queries
const RELATIONSHIP_WORDS: &[&str] = &[
    "where",
    "how",
    "what",
    "when",
    "between",
    "handle",
    "handles",
    "handled",
    "handling",
    "process",
    "processes",
    "processed",
    "processing",
    "flow",
    "flows",
    "manages",
    "managed",
    "managing",
    "connects",
    "connected",
    "connecting",
    "calls",
    "called",
    "calling",
    "triggers",
    "triggered",
    "triggering",
    "uses",
    "used",
    "using",
    "implements",
    "implemented",
    "implementation",
    "creates",
    "created",
    "creating",
    "sends",
    "sent",
    "sending",
    "receives",
    "received",
    "receiving",
];

/// Query type classification
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryType {
    /// Bare identifier, namespace-qualified, or contains uppercase
    /// (e.g., `save_pretrained`, `Client`, `foo::bar`)
    Symbol,
    /// Simple 1-2 word natural query
    SimpleNl,
    /// Multi-word query with relationship indicators
    ComplexNl,
}

/// Query intent detection
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryIntent {
    /// Find/locate something specific (e.g., "find auth handler")
    Find,
    /// How something works (e.g., "how does routing work")
    How,
    /// Where something is defined/located (e.g., "where is config")
    Where,
    /// List/enumerate (e.g., "list all providers")
    List,
    /// General/unspecific
    General,
}

/// Classified query with metadata for strategy selection
#[derive(Debug, Clone)]
pub struct QueryClass {
    /// The detected query type
    pub query_type: QueryType,
    /// Complexity score 0.0 - 1.0
    pub complexity: f32,
    /// Detected query intent
    pub intent: QueryIntent,
    /// Word count
    pub word_count: usize,
    /// Whether relationship words were detected
    pub has_relationship: bool,
    /// Number of relationship word matches
    pub relationship_count: usize,
    /// Detected entity mentions (camelCase/PascalCase words)
    pub entities: Vec<String>,
}

// Compiled regex patterns for symbol detection
static RE_NAMESPACE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[A-Za-z_][A-Za-z0-9_]*(?:::[A-Za-z_][A-Za-z0-9_]*)+$").unwrap());
static RE_LEADING_UNDERSCORE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^_[A-Za-z0-9_]+$").unwrap());
static RE_CAMEL_CASE: LazyLock<Regex> = LazyLock::new(|| {
    // Must start alphabetic, contain at least one uppercase after the first char
    Regex::new(r"^[a-z][a-zA-Z0-9]*[A-Z][a-zA-Z0-9]*$").unwrap()
});
static RE_PASCAL_CASE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[A-Z][A-Za-z0-9]+$").unwrap());
static RE_MIXED_CASE_UNDERSCORE: LazyLock<Regex> = LazyLock::new(|| {
    // e.g., my_var_Name — has underscore AND uppercase
    Regex::new(r"^[A-Za-z][A-Za-z0-9_]*$").unwrap()
});

/// Detect if a query looks like a symbol/identifier
fn is_symbol_query(query: &str) -> bool {
    let trimmed = query.trim();

    if trimmed.is_empty() {
        return false;
    }

    // Namespace-qualified: `foo::bar::Baz`, `std::io::Error`
    if trimmed.contains("::") && trimmed.split_whitespace().count() <= 1 {
        return RE_NAMESPACE.is_match(trimmed);
    }

    // No spaces allowed for remaining symbol patterns
    if trimmed.contains(' ') {
        return false;
    }

    // Leading underscore: `_internal`, `_foo_bar`
    if trimmed.starts_with('_') {
        return RE_LEADING_UNDERSCORE.is_match(trimmed);
    }

    // camelCase: `myFunction`, `authService` (lowercase start, has uppercase after)
    if RE_CAMEL_CASE.is_match(trimmed) {
        return true;
    }

    // PascalCase: `Client`, `Config`, `AuthService`
    if RE_PASCAL_CASE.is_match(trimmed) {
        return true;
    }

    // Mixed case with underscores: `my_var_Name`
    if trimmed.contains('_')
        && trimmed.chars().any(|c| c.is_uppercase())
        && RE_MIXED_CASE_UNDERSCORE.is_match(trimmed)
    {
        return true;
    }

    // Pure snake_case (may contain digits): `save_pretrained`, `h264_decoder`
    if trimmed.contains('_')
        && trimmed
            .chars()
            .all(|c| c.is_lowercase() || c == '_' || c.is_ascii_digit())
        && !trimmed.starts_with('_')
        && trimmed.chars().next().map_or(false, |c| c.is_lowercase())
    {
        return true;
    }

    false
}

/// Detect query intent from query text
fn detect_intent(query: &str) -> QueryIntent {
    let lower = query.to_lowercase();
    let words: Vec<&str> = lower.split_whitespace().collect();

    // Check first word for intent clues
    if let Some(first) = words.first() {
        match *first {
            "find" | "locate" | "search" => return QueryIntent::Find,
            "list" | "show" | "all" => return QueryIntent::List,
            _ => {}
        }
    }

    // Check for question patterns
    if lower.starts_with("how") || lower.starts_with("what is") || lower.starts_with("what does") {
        return QueryIntent::How;
    }
    if lower.starts_with("where") || lower.starts_with("where's") || lower.starts_with("wheres") {
        return QueryIntent::Where;
    }

    QueryIntent::General
}

/// Extract CamelCase/PascalCase entities from query
fn extract_entities(query: &str) -> Vec<String> {
    let mut entities = Vec::new();
    let mut current = String::new();
    let mut prev_upper = false;

    for (i, c) in query.chars().enumerate() {
        if c.is_uppercase()
            && (i == 0
                || prev_upper
                || !current.ends_with('_')
                    && current.chars().last().map_or(false, |p| !p.is_uppercase()))
        {
            if !current.is_empty()
                && (current.chars().last().map_or(false, |p| p.is_lowercase()) || i == 0)
            {
                if !current.is_empty() {
                    entities.push(current.clone());
                }
                current = String::new();
            }
        }
        current.push(c);
        prev_upper = c.is_uppercase();
    }

    if !current.is_empty() && current.chars().any(|c| c.is_uppercase()) {
        entities.push(current);
    }

    entities
}

/// Classify a query and return metadata for strategy selection
pub fn classify(query: &str) -> QueryClass {
    let trimmed = query.trim();
    let words: Vec<&str> = trimmed.split_whitespace().collect();
    let word_count = words.len();
    let query_lower = trimmed.to_lowercase();
    let words_lower: Vec<&str> = query_lower.split_whitespace().collect();

    // Detect query type
    let query_type = if is_symbol_query(trimmed) {
        QueryType::Symbol
    } else if word_count >= 3 {
        QueryType::ComplexNl
    } else {
        QueryType::SimpleNl
    };

    // Detect relationship words
    let relationship_count = RELATIONSHIP_WORDS
        .iter()
        .filter(|w| words_lower.contains(w))
        .count();
    let has_relationship = relationship_count > 0;

    // Calculate complexity (0.0 - 1.0)
    let mut complexity = (word_count as f32 / 10.0).min(1.0);

    // Boost for relationship words
    if has_relationship {
        complexity = (complexity + 0.2).min(1.0);
    }

    // Boost for entities (multiple CamelCase words)
    let entities = extract_entities(trimmed);
    if entities.len() > 1 {
        complexity = (complexity + 0.15).min(1.0);
    }

    // Detect intent
    let intent = detect_intent(trimmed);

    QueryClass {
        query_type,
        complexity,
        intent,
        word_count,
        has_relationship,
        relationship_count,
        entities,
    }
}

/// Search strategy determined by query classification
#[derive(Debug, Clone)]
pub struct SearchStrategy {
    /// BM25 weight for RRF fusion
    pub bm25_weight: f32,
    /// Semantic weight for RRF fusion
    pub semantic_weight: f32,
}

impl SearchStrategy {
    /// Create a BM25-heavy strategy (for symbol queries)
    pub fn bm25_heavy() -> Self {
        Self {
            bm25_weight: 1.0,
            semantic_weight: 0.3,
        }
    }

    /// Create a balanced strategy (for simple NL queries)
    pub fn balanced() -> Self {
        Self {
            bm25_weight: 1.0,
            semantic_weight: 1.0,
        }
    }

    /// Create a semantic-heavy strategy (for complex NL queries)
    pub fn semantic_heavy() -> Self {
        Self {
            bm25_weight: 0.5,
            semantic_weight: 1.0,
        }
    }

    /// Create strategy from query classification
    ///
    /// Uses adaptive alpha based on query type and complexity:
    /// - Symbol queries: BM25-heavy (alpha = 0.3 semantic)
    /// - Simple NL queries: balanced (alpha = 0.5)
    /// - Complex NL queries: semantic-heavy (alpha = 0.6-0.8)
    pub fn from_query(query: &str) -> Self {
        let class = classify(query);

        match class.query_type {
            QueryType::Symbol => Self::bm25_heavy(),
            QueryType::SimpleNl => Self::balanced(),
            QueryType::ComplexNl => {
                // For complex NL, adjust based on complexity and relationship words
                let base = if class.has_relationship || class.intent != QueryIntent::General {
                    // Relationship/question queries need more semantic
                    Self {
                        bm25_weight: 0.4,
                        semantic_weight: 1.0,
                    }
                } else {
                    Self {
                        bm25_weight: 0.6,
                        semantic_weight: 1.0,
                    }
                };

                // Boost semantic further for high complexity
                if class.complexity > 0.7 {
                    Self {
                        bm25_weight: 0.3,
                        semantic_weight: 1.0,
                    }
                } else {
                    base
                }
            }
        }
    }

    /// Apply user-configured weight overrides.
    ///
    /// If the user has set non-default weights in their config, those take
    /// precedence over the adaptive strategy (the user explicitly chose them).
    /// Default config values (0.5/0.5) are treated as "no override".
    pub fn with_config_override(mut self, config_bm25: f32, config_semantic: f32) -> Self {
        const DEFAULT_WEIGHT: f32 = 0.5;
        let is_default = (config_bm25 - DEFAULT_WEIGHT).abs() < f32::EPSILON
            && (config_semantic - DEFAULT_WEIGHT).abs() < f32::EPSILON;

        if !is_default {
            self.bm25_weight = config_bm25;
            self.semantic_weight = config_semantic;
        }

        self
    }

    /// Convert to RrfConfig
    pub fn to_rrf_config(&self) -> super::hybrid::RrfConfig {
        super::hybrid::RrfConfig::with_weights(self.bm25_weight, self.semantic_weight)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_symbol_queries() {
        // Namespace-qualified
        assert!(is_symbol_query("foo::bar::Baz"));
        assert!(is_symbol_query("foo::bar"));
        assert!(is_symbol_query("std::io::Error"));

        // Leading underscore
        assert!(is_symbol_query("_internal"));
        assert!(is_symbol_query("_foo_bar_baz"));

        // camelCase
        assert!(is_symbol_query("myFunction"));
        assert!(is_symbol_query("authService"));

        // PascalCase
        assert!(is_symbol_query("AuthService"));
        assert!(is_symbol_query("Client"));
        assert!(is_symbol_query("Config"));

        // Mixed case with underscores
        assert!(is_symbol_query("my_var_Name"));

        // snake_case
        assert!(is_symbol_query("save_pretrained"));
        assert!(is_symbol_query("h264_decoder"));

        // NOT symbol queries
        assert!(!is_symbol_query("rust error handling"));
        assert!(!is_symbol_query("how does it work"));
        assert!(!is_symbol_query("save pretrained"));
        assert!(!is_symbol_query("std::io error"));
        assert!(!is_symbol_query(""));
        assert!(!is_symbol_query("   "));
    }

    #[test]
    fn test_single_char_and_edge_cases() {
        assert!(!is_symbol_query("a"));
        assert!(!is_symbol_query("1"));
        // Single uppercase letter is too short for PascalCase
        assert!(!is_symbol_query("A"));
    }

    #[test]
    fn test_symbol_with_digits() {
        assert!(is_symbol_query("Http2Client"));
        assert!(is_symbol_query("base64_encode"));
    }

    #[test]
    fn test_classify_symbol() {
        let class = classify("save_pretrained");
        assert_eq!(class.query_type, QueryType::Symbol);

        let class = classify("Client");
        assert_eq!(class.query_type, QueryType::Symbol);

        let class = classify("foo::bar::Baz");
        assert_eq!(class.query_type, QueryType::Symbol);

        let class = classify("std::io::Error");
        assert_eq!(class.query_type, QueryType::Symbol);
    }

    #[test]
    fn test_classify_simple_nl() {
        let class = classify("rust error");
        assert_eq!(class.query_type, QueryType::SimpleNl);
        assert!(!class.has_relationship);

        let class = classify("auth flow");
        assert_eq!(class.query_type, QueryType::SimpleNl);
    }

    #[test]
    fn test_classify_complex_nl() {
        let class = classify("how does the worker handle payloads");
        assert_eq!(class.query_type, QueryType::ComplexNl);
        assert!(class.has_relationship);
        assert!(class.relationship_count >= 2);

        let class = classify("where is the authentication middleware defined");
        assert_eq!(class.query_type, QueryType::ComplexNl);
        assert!(class.has_relationship);
    }

    #[test]
    fn test_classify_empty_and_whitespace() {
        let class = classify("");
        assert_eq!(class.query_type, QueryType::SimpleNl);
        assert_eq!(class.word_count, 0);

        let class = classify("   ");
        assert_eq!(class.query_type, QueryType::SimpleNl);
        assert_eq!(class.word_count, 0);
    }

    #[test]
    fn test_intent_detection() {
        assert_eq!(detect_intent("find auth handler"), QueryIntent::Find);
        assert_eq!(detect_intent("locate the config file"), QueryIntent::Find);
        assert_eq!(detect_intent("list all providers"), QueryIntent::List);
        assert_eq!(detect_intent("how does routing work"), QueryIntent::How);
        assert_eq!(
            detect_intent("where is the main function"),
            QueryIntent::Where
        );
        assert_eq!(detect_intent("rust async tutorial"), QueryIntent::General);
    }

    #[test]
    fn test_search_strategy_from_query() {
        // Symbol queries get BM25-heavy
        let strategy = SearchStrategy::from_query("save_pretrained");
        assert!(strategy.bm25_weight > strategy.semantic_weight);

        // Simple NL gets balanced
        let strategy = SearchStrategy::from_query("rust error");
        assert!((strategy.bm25_weight - strategy.semantic_weight).abs() < 0.1);

        // Complex NL with relationship words gets semantic-heavy
        let strategy = SearchStrategy::from_query("how does the worker handle payloads");
        assert!(strategy.semantic_weight > strategy.bm25_weight);

        // Complex NL with entity mentions
        let strategy = SearchStrategy::from_query("authentication middleware configuration");
        assert!(strategy.semantic_weight >= strategy.bm25_weight);
    }

    #[test]
    fn test_search_strategy_named_constructors() {
        let s = SearchStrategy::bm25_heavy();
        assert_eq!(s.bm25_weight, 1.0);
        assert_eq!(s.semantic_weight, 0.3);

        let s = SearchStrategy::balanced();
        assert_eq!(s.bm25_weight, 1.0);
        assert_eq!(s.semantic_weight, 1.0);

        let s = SearchStrategy::semantic_heavy();
        assert_eq!(s.bm25_weight, 0.5);
        assert_eq!(s.semantic_weight, 1.0);
    }

    #[test]
    fn test_config_override_non_default() {
        let strategy = SearchStrategy::from_query("rust error").with_config_override(0.8, 0.2);
        assert_eq!(strategy.bm25_weight, 0.8);
        assert_eq!(strategy.semantic_weight, 0.2);
    }

    #[test]
    fn test_config_override_default_is_noop() {
        let strategy = SearchStrategy::from_query("save_pretrained").with_config_override(0.5, 0.5);
        // Default config should not override adaptive strategy
        assert_eq!(strategy.bm25_weight, 1.0);
        assert_eq!(strategy.semantic_weight, 0.3);
    }

    #[test]
    fn test_entity_extraction() {
        let entities = extract_entities("auth middleware");
        assert!(
            entities.is_empty() || entities.iter().all(|e| e.chars().any(|c| c.is_uppercase()))
        );

        let entities = extract_entities("ConfigManager");
        assert!(!entities.is_empty());
    }

    #[test]
    fn test_complexity_calculation() {
        let class = classify("a"); // 1 word
        assert!(class.complexity < 0.3);

        let class = classify("how does the worker handle payloads"); // 6 words + relationship
        assert!(class.complexity > 0.5);
    }
}
