//! Task filter query language (P15).
//!
//! A deliberate subset of Todoist's filter syntax, evaluated in memory with
//! zero I/O so it is unit-testable and trustworthy:
//!
//! - combinators: `&`, `|`, `!`, parentheses
//! - `p1`..`p4` — priority
//! - `@label` — has label
//! - `#project` — in project
//! - `overdue`, `today`, `no date`, `no label`, `subtask`
//! - `due before: YYYY-MM-DD`, `due after: YYYY-MM-DD`
//! - `search: text` — substring match over title + description
//! - status atoms: `pending`, `in_progress`, `done`, `cancelled`, `blocked`
//!   (`blocked` matches the *derived* blocked flag, never a stored value)
//!
//! Dates are structured ISO (`YYYY-MM-DD`) by design — an agent emits
//! structured dates; natural-language parsing is a human affordance we
//! deliberately do not build (ROADMAP P15).

use crate::Task;

/// Maximum filter input length in bytes.
///
/// Bound justification: a filter arrives as a single tool-call argument the
/// model must emit within one response; useful filters are tens of bytes.
/// 4096 caps worst-case lexer work without ever rejecting a real query.
pub const FILTER_INPUT_MAX_BYTES: usize = 4096;

/// Maximum expression nesting depth.
///
/// Bound justification: the parser recurses once per nesting level; 32 levels
/// is far beyond any human- or model-written filter and keeps worst-case
/// stack use trivially small.
pub const FILTER_DEPTH_MAX: usize = 32;

/// Parsed filter expression tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilterExpr {
    And(Box<FilterExpr>, Box<FilterExpr>),
    Or(Box<FilterExpr>, Box<FilterExpr>),
    Not(Box<FilterExpr>),
    Atom(FilterAtom),
}

/// A single filter predicate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilterAtom {
    /// `p1`..`p4`
    Priority(i64),
    /// `@label`
    Label(String),
    /// `#project`
    Project(String),
    /// `overdue` — due date strictly before today and not done/cancelled
    Overdue,
    /// `today` — due date equals today
    Today,
    /// `no date`
    NoDate,
    /// `no label`
    NoLabel,
    /// `subtask` — has a parent
    Subtask,
    /// `due before: YYYY-MM-DD` (exclusive)
    DueBefore(String),
    /// `due after: YYYY-MM-DD` (exclusive)
    DueAfter(String),
    /// `search: text` — case-insensitive substring over title + description
    Search(String),
    /// `pending` | `in_progress` | `done` | `cancelled`
    Status(String),
    /// `blocked` — derived from dependencies at read time
    Blocked,
}

/// Filter parse error with position context.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FilterError {
    #[error("filter is empty")]
    Empty,
    #[error("filter exceeds {FILTER_INPUT_MAX_BYTES} bytes (got {0})")]
    TooLong(usize),
    #[error("filter nesting exceeds {FILTER_DEPTH_MAX} levels")]
    TooDeep,
    #[error("unexpected token '{0}'")]
    UnexpectedToken(String),
    #[error("unexpected end of filter (expected {0})")]
    UnexpectedEnd(&'static str),
    #[error("invalid date '{0}' (expected YYYY-MM-DD)")]
    InvalidDate(String),
    #[error("unknown keyword '{0}'")]
    UnknownKeyword(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    Amp,
    Pipe,
    Bang,
    LParen,
    RParen,
    Atom(FilterAtom),
}

/// Parse a filter query into an expression tree.
pub fn parse(input: &str) -> Result<FilterExpr, FilterError> {
    if input.len() > FILTER_INPUT_MAX_BYTES {
        return Err(FilterError::TooLong(input.len()));
    }
    let tokens = lex(input)?;
    if tokens.is_empty() {
        return Err(FilterError::Empty);
    }
    let mut parser = Parser { tokens, pos: 0 };
    let expr = parser.parse_or(0)?;
    if parser.pos < parser.tokens.len() {
        return Err(FilterError::UnexpectedToken(format!(
            "{:?}",
            parser.tokens[parser.pos]
        )));
    }
    Ok(expr)
}

impl FilterExpr {
    /// Evaluate this filter against a task.
    ///
    /// `today` is the current date as `YYYY-MM-DD`, passed in so evaluation is
    /// pure. Date comparisons are lexicographic, which is correct for ISO
    /// dates.
    #[must_use]
    pub fn matches(&self, task: &Task, today: &str) -> bool {
        match self {
            Self::And(a, b) => a.matches(task, today) && b.matches(task, today),
            Self::Or(a, b) => a.matches(task, today) || b.matches(task, today),
            Self::Not(inner) => !inner.matches(task, today),
            Self::Atom(atom) => atom.matches(task, today),
        }
    }
}

impl FilterAtom {
    fn matches(&self, task: &Task, today: &str) -> bool {
        match self {
            Self::Priority(p) => task.priority == *p,
            Self::Label(label) => task.labels.iter().any(|l| l.eq_ignore_ascii_case(label)),
            Self::Project(project) => task
                .project
                .as_deref()
                .is_some_and(|p| p.eq_ignore_ascii_case(project)),
            Self::Overdue => {
                task.status != "done"
                    && task.status != "cancelled"
                    && due_date(task).is_some_and(|d| d.as_str() < today)
            }
            Self::Today => due_date(task).is_some_and(|d| d == today),
            Self::NoDate => task.due_at.is_none(),
            Self::NoLabel => task.labels.is_empty(),
            Self::Subtask => task.parent_id.is_some(),
            Self::DueBefore(date) => due_date(task).is_some_and(|d| d < *date),
            Self::DueAfter(date) => due_date(task).is_some_and(|d| d > *date),
            Self::Search(text) => {
                let needle = text.to_lowercase();
                task.title.to_lowercase().contains(&needle)
                    || task
                        .description
                        .as_deref()
                        .is_some_and(|d| d.to_lowercase().contains(&needle))
            }
            Self::Status(status) => task.status == *status,
            Self::Blocked => task.blocked,
        }
    }
}

/// Extract the date part (`YYYY-MM-DD`) of a task's due timestamp.
fn due_date(task: &Task) -> Option<String> {
    task.due_at
        .as_deref()
        .map(|d| d.chars().take(10).collect::<String>())
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn parse_or(&mut self, depth: usize) -> Result<FilterExpr, FilterError> {
        if depth >= FILTER_DEPTH_MAX {
            return Err(FilterError::TooDeep);
        }
        let mut left = self.parse_and(depth + 1)?;
        while self.peek() == Some(&Token::Pipe) {
            self.pos += 1;
            let right = self.parse_and(depth + 1)?;
            left = FilterExpr::Or(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_and(&mut self, depth: usize) -> Result<FilterExpr, FilterError> {
        if depth >= FILTER_DEPTH_MAX {
            return Err(FilterError::TooDeep);
        }
        let mut left = self.parse_unary(depth + 1)?;
        while self.peek() == Some(&Token::Amp) {
            self.pos += 1;
            let right = self.parse_unary(depth + 1)?;
            left = FilterExpr::And(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_unary(&mut self, depth: usize) -> Result<FilterExpr, FilterError> {
        if depth >= FILTER_DEPTH_MAX {
            return Err(FilterError::TooDeep);
        }
        match self.peek() {
            Some(Token::Bang) => {
                self.pos += 1;
                let inner = self.parse_unary(depth + 1)?;
                Ok(FilterExpr::Not(Box::new(inner)))
            }
            Some(Token::LParen) => {
                self.pos += 1;
                let inner = self.parse_or(depth + 1)?;
                match self.peek() {
                    Some(Token::RParen) => {
                        self.pos += 1;
                        Ok(inner)
                    }
                    _ => Err(FilterError::UnexpectedEnd("')'")),
                }
            }
            Some(Token::Atom(_)) => {
                let Some(Token::Atom(atom)) = self.tokens.get(self.pos).cloned() else {
                    return Err(FilterError::UnexpectedEnd("atom"));
                };
                self.pos += 1;
                Ok(FilterExpr::Atom(atom))
            }
            Some(other) => Err(FilterError::UnexpectedToken(format!("{other:?}"))),
            None => Err(FilterError::UnexpectedEnd("expression")),
        }
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }
}

fn lex(input: &str) -> Result<Vec<Token>, FilterError> {
    let mut tokens = Vec::new();
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0;
    // Bounded by input length: every iteration consumes at least one char.
    while i < chars.len() {
        let c = chars[i];
        match c {
            ' ' | '\t' | '\n' | '\r' => i += 1,
            '&' => {
                tokens.push(Token::Amp);
                i += 1;
            }
            '|' => {
                tokens.push(Token::Pipe);
                i += 1;
            }
            '!' => {
                tokens.push(Token::Bang);
                i += 1;
            }
            '(' => {
                tokens.push(Token::LParen);
                i += 1;
            }
            ')' => {
                tokens.push(Token::RParen);
                i += 1;
            }
            '@' => {
                let (word, next) = take_word(&chars, i + 1);
                if word.is_empty() {
                    return Err(FilterError::UnknownKeyword("@".to_string()));
                }
                tokens.push(Token::Atom(FilterAtom::Label(word)));
                i = next;
            }
            '#' => {
                let (word, next) = take_word(&chars, i + 1);
                if word.is_empty() {
                    return Err(FilterError::UnknownKeyword("#".to_string()));
                }
                tokens.push(Token::Atom(FilterAtom::Project(word)));
                i = next;
            }
            _ => {
                let word_start = i;
                let (word, next) = take_word(&chars, i);
                if word.is_empty() {
                    return Err(FilterError::UnexpectedToken(c.to_string()));
                }
                let lower = word.to_lowercase();
                // `search:` swallows free text up to the next operator; accept
                // both `search: foo` and `search:foo`.
                if lower.starts_with("search:") {
                    let (text, after) = take_until_operator(&chars, word_start);
                    let body = text.get("search:".len()..).unwrap_or("").trim();
                    if body.is_empty() {
                        return Err(FilterError::UnexpectedEnd("search text"));
                    }
                    tokens.push(Token::Atom(FilterAtom::Search(body.to_string())));
                    i = after;
                    continue;
                }
                i = next;
                match lower.as_str() {
                    "p1" => tokens.push(Token::Atom(FilterAtom::Priority(1))),
                    "p2" => tokens.push(Token::Atom(FilterAtom::Priority(2))),
                    "p3" => tokens.push(Token::Atom(FilterAtom::Priority(3))),
                    "p4" => tokens.push(Token::Atom(FilterAtom::Priority(4))),
                    "overdue" => tokens.push(Token::Atom(FilterAtom::Overdue)),
                    "today" => tokens.push(Token::Atom(FilterAtom::Today)),
                    "subtask" => tokens.push(Token::Atom(FilterAtom::Subtask)),
                    "blocked" => tokens.push(Token::Atom(FilterAtom::Blocked)),
                    "pending" | "in_progress" | "done" | "cancelled" => {
                        tokens.push(Token::Atom(FilterAtom::Status(lower)));
                    }
                    "no" => {
                        let (follow, after) = take_word(&chars, skip_ws(&chars, i));
                        match follow.to_lowercase().as_str() {
                            "date" => tokens.push(Token::Atom(FilterAtom::NoDate)),
                            "label" => tokens.push(Token::Atom(FilterAtom::NoLabel)),
                            other => {
                                return Err(FilterError::UnknownKeyword(format!("no {other}")));
                            }
                        }
                        i = after;
                    }
                    "due" => {
                        // Accept `due before: DATE` / `due after:DATE` — the
                        // colon may or may not be followed by a space.
                        let (text, after) = take_until_operator(&chars, i);
                        i = after;
                        let lower_rest = text.to_lowercase();
                        let (build, raw_date): (fn(String) -> FilterAtom, &str) =
                            if lower_rest.starts_with("before:") {
                                (
                                    FilterAtom::DueBefore,
                                    text.get("before:".len()..).unwrap_or(""),
                                )
                            } else if lower_rest.starts_with("after:") {
                                (
                                    FilterAtom::DueAfter,
                                    text.get("after:".len()..).unwrap_or(""),
                                )
                            } else {
                                return Err(FilterError::UnknownKeyword(format!("due {text}")));
                            };
                        let date = raw_date.trim().to_string();
                        if !is_iso_date(&date) {
                            return Err(FilterError::InvalidDate(date));
                        }
                        tokens.push(Token::Atom(build(date)));
                    }
                    other => return Err(FilterError::UnknownKeyword(other.to_string())),
                }
            }
        }
    }
    Ok(tokens)
}

fn skip_ws(chars: &[char], start: usize) -> usize {
    let mut i = start;
    while i < chars.len() && chars[i].is_whitespace() {
        i += 1;
    }
    i
}

/// Consume a word: non-whitespace chars that are not operators/parens.
/// A trailing ':' is kept so `search:` / `before:` lex as single words.
fn take_word(chars: &[char], start: usize) -> (String, usize) {
    let mut i = start;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() || matches!(c, '&' | '|' | '!' | '(' | ')') {
            break;
        }
        i += 1;
    }
    (chars[start..i].iter().collect(), i)
}

/// Consume free text until an operator, closing paren, or end of input.
fn take_until_operator(chars: &[char], start: usize) -> (String, usize) {
    let mut i = start;
    while i < chars.len() && !matches!(chars[i], '&' | '|' | ')') {
        i += 1;
    }
    let text: String = chars[start..i].iter().collect();
    (text.trim().to_string(), i)
}

/// Strict `YYYY-MM-DD` shape check (structured dates only, by design).
fn is_iso_date(s: &str) -> bool {
    let bytes = s.as_bytes();
    bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(idx, b)| matches!(idx, 4 | 7) || b.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task() -> Task {
        Task {
            id: 1,
            parent_id: None,
            scope: "session".to_string(),
            scope_id: Some("s1".to_string()),
            project: Some("harness".to_string()),
            title: "Wire the acceptance runner".to_string(),
            description: Some("run command checks from the harness".to_string()),
            status: "pending".to_string(),
            priority: 2,
            labels: vec!["rust".to_string(), "p14".to_string()],
            tool_scope: vec![],
            due_at: Some("2026-07-20".to_string()),
            recurrence: None,
            depends_on: vec![],
            acceptance: None,
            assignee: None,
            sort_order: 0,
            created_at: String::new(),
            updated_at: String::new(),
            completed_at: None,
            blocked: false,
        }
    }

    const TODAY: &str = "2026-07-18";

    #[test]
    fn priority_atom_matches_exact_priority() {
        let expr = parse("p2").unwrap();
        assert!(expr.matches(&task(), TODAY));
        assert!(!parse("p1").unwrap().matches(&task(), TODAY));
    }

    #[test]
    fn label_atom_is_case_insensitive() {
        assert!(parse("@RUST").unwrap().matches(&task(), TODAY));
        assert!(!parse("@missing").unwrap().matches(&task(), TODAY));
    }

    #[test]
    fn project_atom_matches() {
        assert!(parse("#harness").unwrap().matches(&task(), TODAY));
        assert!(!parse("#other").unwrap().matches(&task(), TODAY));
    }

    #[test]
    fn and_or_not_and_parens_compose() {
        let expr = parse("(p1 | p2) & !done & @rust").unwrap();
        assert!(expr.matches(&task(), TODAY));
        let expr2 = parse("(p1 | p3) & @rust").unwrap();
        assert!(!expr2.matches(&task(), TODAY));
    }

    #[test]
    fn and_binds_tighter_than_or() {
        // p1 | p2 & @missing => p1 | (p2 & @missing) => false for our task
        // (p2 matches but @missing fails; p1 fails)
        let expr = parse("p1 | p2 & @missing").unwrap();
        assert!(!expr.matches(&task(), TODAY));
        // (p1 | p2) & @missing would also be false; distinguish via a case
        // where grouping flips the result: p2 | p1 & @missing => p2 => true
        let expr2 = parse("p2 | p1 & @missing").unwrap();
        assert!(expr2.matches(&task(), TODAY));
    }

    #[test]
    fn overdue_requires_past_due_and_open_status() {
        let mut t = task();
        t.due_at = Some("2026-07-10".to_string());
        assert!(parse("overdue").unwrap().matches(&t, TODAY));
        t.status = "done".to_string();
        assert!(
            !parse("overdue").unwrap().matches(&t, TODAY),
            "done tasks are never overdue"
        );
        t.status = "pending".to_string();
        t.due_at = Some(TODAY.to_string());
        assert!(
            !parse("overdue").unwrap().matches(&t, TODAY),
            "due today is not overdue"
        );
    }

    #[test]
    fn today_matches_only_exact_date() {
        let mut t = task();
        t.due_at = Some(TODAY.to_string());
        assert!(parse("today").unwrap().matches(&t, TODAY));
        t.due_at = Some("2026-07-19".to_string());
        assert!(!parse("today").unwrap().matches(&t, TODAY));
    }

    #[test]
    fn no_date_and_no_label_match_absence() {
        let mut t = task();
        t.due_at = None;
        t.labels.clear();
        assert!(parse("no date").unwrap().matches(&t, TODAY));
        assert!(parse("no label").unwrap().matches(&t, TODAY));
        assert!(!parse("no date").unwrap().matches(&task(), TODAY));
        assert!(!parse("no label").unwrap().matches(&task(), TODAY));
    }

    #[test]
    fn due_before_and_after_are_exclusive() {
        let t = task(); // due 2026-07-20
        assert!(parse("due before: 2026-07-21").unwrap().matches(&t, TODAY));
        assert!(!parse("due before: 2026-07-20").unwrap().matches(&t, TODAY));
        assert!(parse("due after: 2026-07-19").unwrap().matches(&t, TODAY));
        assert!(!parse("due after: 2026-07-20").unwrap().matches(&t, TODAY));
    }

    #[test]
    fn due_comparisons_ignore_time_component() {
        let mut t = task();
        t.due_at = Some("2026-07-20 15:30:00".to_string());
        assert!(parse("due before: 2026-07-21").unwrap().matches(&t, TODAY));
    }

    #[test]
    fn search_is_case_insensitive_substring_over_title_and_description() {
        assert!(parse("search: ACCEPTANCE").unwrap().matches(&task(), TODAY));
        assert!(
            parse("search: command checks")
                .unwrap()
                .matches(&task(), TODAY)
        );
        assert!(!parse("search: nomatch").unwrap().matches(&task(), TODAY));
    }

    #[test]
    fn search_text_stops_at_operator() {
        let expr = parse("search: acceptance & p2").unwrap();
        assert!(expr.matches(&task(), TODAY));
    }

    #[test]
    fn search_and_due_accept_no_space_after_colon() {
        assert!(parse("search:acceptance").unwrap().matches(&task(), TODAY));
        assert!(
            parse("due before:2026-07-21")
                .unwrap()
                .matches(&task(), TODAY)
        );
        assert!(
            parse("due after:2026-07-19")
                .unwrap()
                .matches(&task(), TODAY)
        );
    }

    #[test]
    fn empty_search_text_is_rejected() {
        assert!(matches!(
            parse("search:"),
            Err(FilterError::UnexpectedEnd(_))
        ));
    }

    #[test]
    fn subtask_matches_only_children() {
        assert!(!parse("subtask").unwrap().matches(&task(), TODAY));
        let mut t = task();
        t.parent_id = Some(9);
        assert!(parse("subtask").unwrap().matches(&t, TODAY));
    }

    #[test]
    fn status_atoms_match_stored_status() {
        assert!(parse("pending").unwrap().matches(&task(), TODAY));
        let mut t = task();
        t.status = "in_progress".to_string();
        assert!(parse("in_progress").unwrap().matches(&t, TODAY));
        assert!(!parse("done").unwrap().matches(&t, TODAY));
    }

    #[test]
    fn blocked_matches_derived_flag_only() {
        let mut t = task();
        assert!(!parse("blocked").unwrap().matches(&t, TODAY));
        t.blocked = true;
        assert!(parse("blocked").unwrap().matches(&t, TODAY));
    }

    #[test]
    fn empty_filter_is_rejected() {
        assert_eq!(parse(""), Err(FilterError::Empty));
        assert_eq!(parse("   "), Err(FilterError::Empty));
    }

    #[test]
    fn unknown_keyword_is_rejected() {
        assert!(matches!(
            parse("banana"),
            Err(FilterError::UnknownKeyword(_))
        ));
        assert!(matches!(
            parse("no banana"),
            Err(FilterError::UnknownKeyword(_))
        ));
        assert!(matches!(
            parse("due soon: x"),
            Err(FilterError::UnknownKeyword(_))
        ));
    }

    #[test]
    fn malformed_date_is_rejected() {
        assert!(matches!(
            parse("due before: tomorrow"),
            Err(FilterError::InvalidDate(_))
        ));
        assert!(matches!(
            parse("due before: 2026-7-1"),
            Err(FilterError::InvalidDate(_))
        ));
    }

    #[test]
    fn unbalanced_parens_are_rejected() {
        assert!(matches!(
            parse("(p1 & p2"),
            Err(FilterError::UnexpectedEnd(_))
        ));
        assert!(matches!(parse("p1)"), Err(FilterError::UnexpectedToken(_))));
    }

    #[test]
    fn trailing_operator_is_rejected() {
        assert!(matches!(parse("p1 &"), Err(FilterError::UnexpectedEnd(_))));
        assert!(matches!(
            parse("| p1"),
            Err(FilterError::UnexpectedToken(_))
        ));
    }

    #[test]
    fn over_long_input_is_rejected() {
        let long = "p1 & ".repeat(FILTER_INPUT_MAX_BYTES / 4);
        assert!(matches!(parse(&long), Err(FilterError::TooLong(_))));
    }

    #[test]
    fn over_deep_nesting_is_rejected() {
        let nested = format!("{}p1{}", "(".repeat(64), ")".repeat(64));
        assert_eq!(parse(&nested), Err(FilterError::TooDeep));
    }

    #[test]
    fn deep_but_legal_nesting_parses() {
        let nested = format!("{}p2{}", "(".repeat(8), ")".repeat(8));
        assert!(parse(&nested).unwrap().matches(&task(), TODAY));
    }

    #[test]
    fn double_negation_round_trips() {
        assert!(parse("!!p2").unwrap().matches(&task(), TODAY));
        assert!(!parse("!p2").unwrap().matches(&task(), TODAY));
    }
}
