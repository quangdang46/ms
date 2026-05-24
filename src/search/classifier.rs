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

const _STOPWORDS: &[&str] = &[
    "a", "an", "and", "are", "as", "at", "be", "by", "do", "does", "for", "from", "has", "have",
    "how", "if", "in", "is", "it", "not", "of", "on", "or", "the", "to", "was", "what", "when",
    "where", "which", "who", "why", "with",
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

/// Symbol query regex patterns
const SYMBOL_PATTERN_1: &str = r"^[A-Za-z_][A-Za-z0-9_]*(?:::[A-Za-z_][A-Za-z0-9_]*)+$";
const SYMBOL_PATTERN_2: &str = r"^_[A-Za-z0-9_]+$";
const SYMBOL_PATTERN_3: &str = r"^[A-Za-z][A-Za-z0-9]*[A-Z_][A-Za-z0-9_]*$";
const SYMBOL_PATTERN_4: &str = r"^[A-Z][A-Za-z0-9]+$";

/// Detect if a query looks like a symbol/identifier
fn is_symbol_query(query: &str) -> bool {
    let trimmed = query.trim();

    // Check each pattern
    if trimmed.matches("::").count() >= 1 && !trimmed.split_whitespace().count() > 1 {
        // Namespace-qualified: `foo::bar::Baz`
        return regex_matches(trimmed, SYMBOL_PATTERN_1);
    }
    if trimmed.starts_with('_') && !trimmed.contains(' ') {
        // Leading underscore: `_internal`, `_foo_bar`
        return regex_matches(trimmed, SYMBOL_PATTERN_2);
    }
    if trimmed.contains(|c: char| c.is_uppercase()) && !trimmed.contains(' ') {
        // Contains uppercase (camelCase/PascalCase): `myFunction`, `AuthService`
        return regex_matches(trimmed, SYMBOL_PATTERN_3);
    }
    if trimmed.chars().next().map_or(false, |c| c.is_uppercase()) && !trimmed.contains(' ') {
        // Starts with uppercase: `Client`, `Config`
        return regex_matches(trimmed, SYMBOL_PATTERN_4);
    }

    false
}

/// Simple regex matching helper (avoids external dependency)
fn regex_matches(text: &str, pattern: &str) -> bool {
    // For simplicity, implement basic pattern matching
    // In production, use the `regex` crate
    match pattern {
        p if p == SYMBOL_PATTERN_1 => {
            // ^[A-Za-z_][A-Za-z0-9_]*(?:::[A-Za-z_][A-Za-z0-9_]*)+$
            let parts: Vec<&str> = text.split("::").collect();
            if parts.len() < 2 {
                return false;
            }
            parts.iter().all(|p| {
                !p.is_empty()
                    && p.chars()
                        .next()
                        .map_or(false, |c| c.is_alphabetic() || c == '_')
                    && p.chars().all(|c| c.is_alphanumeric() || c == '_')
            })
        }
        p if p == SYMBOL_PATTERN_2 => {
            // ^_[A-Za-z0-9_]+$
            text.starts_with('_')
                && text.len() > 1
                && text.chars().all(|c| c.is_alphanumeric() || c == '_')
        }
        p if p == SYMBOL_PATTERN_3 => {
            // ^[A-Za-z][A-Za-z0-9]*[A-Z_][A-Za-z0-9_]*$
            let has_upper_or_underscore = text.chars().any(|c| c.is_uppercase() || c == '_');
            text.chars().next().map_or(false, |c| c.is_alphabetic())
                && text.chars().all(|c| c.is_alphanumeric() || c == '_')
                && has_upper_or_underscore
        }
        p if p == SYMBOL_PATTERN_4 => {
            // ^[A-Z][A-Za-z0-9]+$
            text.chars().next().map_or(false, |c| c.is_uppercase())
                && text.len() > 1
                && text.chars().all(|c| c.is_alphanumeric())
        }
        _ => false,
    }
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

        // Leading underscore
        assert!(is_symbol_query("_internal"));
        assert!(is_symbol_query("_foo_bar_baz"));

        // Contains uppercase
        assert!(is_symbol_query("myFunction"));
        assert!(is_symbol_query("AuthService"));
        assert!(is_symbol_query("my_var_Name"));

        // Starts with uppercase
        assert!(is_symbol_query("Client"));
        assert!(is_symbol_query("Config"));

        // NOT symbol queries
        assert!(!is_symbol_query("rust error handling"));
        assert!(!is_symbol_query("how does it work"));
        assert!(!is_symbol_query("save pretrained")); // lowercase
    }

    #[test]
    fn test_classify_symbol() {
        let class = classify("save_pretrained");
        assert_eq!(class.query_type, QueryType::Symbol);

        let class = classify("Client");
        assert_eq!(class.query_type, QueryType::Symbol);

        let class = classify("foo::bar::Baz");
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
    fn test_entity_extraction() {
        let entities = extract_entities("auth middleware");
        assert!(
            entities.is_empty() || entities.iter().all(|e| e.chars().any(|c| c.is_uppercase()))
        );

        let entities = extract_entities("ConfigManager");
        assert!(!entities.is_empty());

        let entities = extract_entities("how does the worker handle payloads");
        // May contain "Worker", "Payloads" etc.
        for e in &entities {
            assert!(
                e.chars().any(|c| c.is_uppercase()),
                "Entity '{}' should have uppercase",
                e
            );
        }
    }

    #[test]
    fn test_complexity_calculation() {
        let class = classify("a"); // 1 word
        assert!(class.complexity < 0.3);

        let class = classify("how does the worker handle payloads"); // 6 words + relationship
        assert!(class.complexity > 0.5);
    }
}
