use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::{self, BufRead};
use std::path::PathBuf;

use chrono::{NaiveDate, NaiveDateTime};
use serde::Deserialize;
use serde_json::Value;

// ---------------------------------------------------------------------------
// Raw JSONL structures (loosely typed to handle variation)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct SessionLine {
    #[serde(rename = "type")]
    line_type: Option<String>,
    timestamp: Option<String>,
    #[serde(rename = "sessionId")]
    session_id: Option<String>,
    message: Option<MessagePayload>,
}

#[derive(Deserialize)]
struct MessagePayload {
    model: Option<String>,
    usage: Option<Usage>,
    content: Option<Value>,
}

#[derive(Deserialize)]
struct Usage {
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    cache_read_input_tokens: Option<u64>,
    cache_creation_input_tokens: Option<u64>,
}

// ---------------------------------------------------------------------------
// Aggregated metrics
// ---------------------------------------------------------------------------

#[derive(Default, Clone)]
pub struct DayTokens {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_creation: u64,
}

impl DayTokens {
    pub fn total(&self) -> u64 {
        self.input + self.output + self.cache_read + self.cache_creation
    }
}

/// Per-session tracking during parse
#[derive(Default)]
struct SessionAccum {
    first_ts: Option<NaiveDateTime>,
    last_ts: Option<NaiveDateTime>,
    user_turns: u32,
    assistant_turns: u32,
    files_touched: HashMap<String, ()>,
}

/// Session-level stats after aggregation
#[derive(Clone)]
pub struct SessionStats {
    pub session_id: String,
    pub first_date: Option<NaiveDate>,
    pub duration_secs: u64,
    pub user_turns: u32,
    pub assistant_turns: u32,
    pub files_touched_count: usize,
}

/// Per-model token totals for cost estimation
#[derive(Default, Clone)]
pub struct ModelTokens {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_creation: u64,
}

/// All metrics collected from JSONL files
pub struct Metrics {
    /// Tokens per day
    pub daily_tokens: BTreeMap<NaiveDate, DayTokens>,
    /// Activity counts per hour of day (0..24)
    pub hour_counts: [u64; 24],
    /// Tool call counts by tool name
    pub tool_calls: BTreeMap<String, u64>,
    /// File operations: (created, edited, read)
    pub file_ops: FileOps,
    /// Edit count per file path (for "most edited file")
    pub file_edit_counts: BTreeMap<String, u64>,
    /// Per-session stats (sorted by duration desc)
    pub sessions: Vec<SessionStats>,
    /// Total conversation turns (user messages)
    pub total_user_turns: u32,
    pub total_assistant_turns: u32,
    /// Per-model token totals for cost estimation
    pub model_tokens: BTreeMap<String, ModelTokens>,
    /// Per-day per-model token totals (for accurate daily cost estimates)
    pub daily_model_tokens: BTreeMap<NaiveDate, BTreeMap<String, ModelTokens>>,
}

#[derive(Default, Clone)]
pub struct FileOps {
    pub created: u64,
    pub edited: u64,
    pub read: u64,
}

// ---------------------------------------------------------------------------
// Data collection
// ---------------------------------------------------------------------------

pub fn collect_jsonl_files() -> Vec<PathBuf> {
    let Some(home) = dirs::home_dir() else {
        return vec![];
    };
    let projects_dir = home.join(".claude").join("projects");
    if !projects_dir.is_dir() {
        return vec![];
    }

    let pattern = projects_dir
        .join("**")
        .join("*.jsonl")
        .to_string_lossy()
        .to_string();
    glob::glob(&pattern)
        .map(|paths| paths.filter_map(|p| p.ok()).collect())
        .unwrap_or_default()
}

pub fn parse_all(files: &[PathBuf]) -> Metrics {
    let mut daily_tokens: BTreeMap<NaiveDate, DayTokens> = BTreeMap::new();
    let mut hour_counts = [0u64; 24];
    let mut tool_calls: BTreeMap<String, u64> = BTreeMap::new();
    let mut file_ops = FileOps::default();
    let mut file_edit_counts: BTreeMap<String, u64> = BTreeMap::new();
    let mut sessions: HashMap<String, SessionAccum> = HashMap::new();
    let mut total_user_turns: u32 = 0;
    let mut total_assistant_turns: u32 = 0;
    let mut model_tokens: BTreeMap<String, ModelTokens> = BTreeMap::new();
    let mut daily_model_tokens: BTreeMap<NaiveDate, BTreeMap<String, ModelTokens>> =
        BTreeMap::new();

    // Track which files have been written (to distinguish create vs edit)
    let mut written_files: HashMap<String, u64> = HashMap::new();

    for path in files {
        let Ok(file) = fs::File::open(path) else {
            continue;
        };
        let reader = io::BufReader::new(file);
        for line in reader.lines() {
            let Ok(line) = line else { continue };
            let Ok(entry) = serde_json::from_str::<SessionLine>(&line) else {
                continue;
            };

            let line_type = entry.line_type.as_deref().unwrap_or("");
            let ts_str = entry.timestamp.as_deref().unwrap_or("");
            let ts_dt = parse_datetime(ts_str);
            let ts_date = parse_date(ts_str);
            let sid = entry.session_id.clone().unwrap_or_default();

            // Track session timestamps
            if !sid.is_empty() {
                if let Some(dt) = ts_dt {
                    let sess = sessions.entry(sid.clone()).or_default();
                    match sess.first_ts {
                        None => sess.first_ts = Some(dt),
                        Some(first) if dt < first => sess.first_ts = Some(dt),
                        _ => {}
                    }
                    match sess.last_ts {
                        None => sess.last_ts = Some(dt),
                        Some(last) if dt > last => sess.last_ts = Some(dt),
                        _ => {}
                    }
                }
            }

            // Hour-of-day heatmap
            if let Some(dt) = ts_dt {
                if line_type == "user" || line_type == "assistant" {
                    let hour = dt.time().hour() as usize;
                    if hour < 24 {
                        hour_counts[hour] += 1;
                    }
                }
            }

            match line_type {
                "assistant" => {
                    total_assistant_turns += 1;
                    if !sid.is_empty() {
                        sessions.entry(sid.clone()).or_default().assistant_turns += 1;
                    }

                    let Some(ref message) = entry.message else {
                        continue;
                    };

                    // Token usage
                    if let Some(ref usage) = message.usage {
                        if let Some(date) = ts_date {
                            let day = daily_tokens.entry(date).or_default();
                            day.input += usage.input_tokens.unwrap_or(0);
                            day.output += usage.output_tokens.unwrap_or(0);
                            day.cache_read += usage.cache_read_input_tokens.unwrap_or(0);
                            day.cache_creation += usage.cache_creation_input_tokens.unwrap_or(0);
                        }

                        // Per-model tracking
                        if let Some(ref model) = message.model {
                            let model_key = normalize_model(model);
                            if !model_key.is_empty() {
                                let mt = model_tokens.entry(model_key.clone()).or_default();
                                mt.input += usage.input_tokens.unwrap_or(0);
                                mt.output += usage.output_tokens.unwrap_or(0);
                                mt.cache_read += usage.cache_read_input_tokens.unwrap_or(0);
                                mt.cache_creation += usage.cache_creation_input_tokens.unwrap_or(0);

                                // Per-day per-model tracking
                                if let Some(date) = ts_date {
                                    let dmt = daily_model_tokens
                                        .entry(date)
                                        .or_default()
                                        .entry(model_key)
                                        .or_default();
                                    dmt.input += usage.input_tokens.unwrap_or(0);
                                    dmt.output += usage.output_tokens.unwrap_or(0);
                                    dmt.cache_read += usage.cache_read_input_tokens.unwrap_or(0);
                                    dmt.cache_creation +=
                                        usage.cache_creation_input_tokens.unwrap_or(0);
                                }
                            }
                        }
                    }

                    // Tool calls from content blocks
                    if let Some(Value::Array(ref blocks)) = message.content {
                        for block in blocks {
                            if block.get("type").and_then(|v| v.as_str()) == Some("tool_use") {
                                let tool_name = block
                                    .get("name")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("unknown");

                                *tool_calls.entry(tool_name.to_string()).or_insert(0) += 1;

                                let input = block.get("input");

                                match tool_name {
                                    "Write" => {
                                        if let Some(fp) = input
                                            .and_then(|i| i.get("file_path"))
                                            .and_then(|v| v.as_str())
                                        {
                                            let count =
                                                written_files.entry(fp.to_string()).or_insert(0);
                                            if *count == 0 {
                                                file_ops.created += 1;
                                            } else {
                                                file_ops.edited += 1;
                                            }
                                            *count += 1;
                                            *file_edit_counts
                                                .entry(shorten_path(fp).to_string())
                                                .or_insert(0) += 1;

                                            if !sid.is_empty() {
                                                sessions
                                                    .entry(sid.clone())
                                                    .or_default()
                                                    .files_touched
                                                    .insert(fp.to_string(), ());
                                            }
                                        }
                                    }
                                    "Edit" => {
                                        file_ops.edited += 1;
                                        if let Some(fp) = input
                                            .and_then(|i| i.get("file_path"))
                                            .and_then(|v| v.as_str())
                                        {
                                            *file_edit_counts
                                                .entry(shorten_path(fp).to_string())
                                                .or_insert(0) += 1;
                                            *written_files.entry(fp.to_string()).or_insert(0) += 1;

                                            if !sid.is_empty() {
                                                sessions
                                                    .entry(sid.clone())
                                                    .or_default()
                                                    .files_touched
                                                    .insert(fp.to_string(), ());
                                            }
                                        }
                                    }
                                    "Read" => {
                                        file_ops.read += 1;
                                        if let Some(fp) = input
                                            .and_then(|i| i.get("file_path"))
                                            .and_then(|v| v.as_str())
                                        {
                                            if !sid.is_empty() {
                                                sessions
                                                    .entry(sid.clone())
                                                    .or_default()
                                                    .files_touched
                                                    .insert(fp.to_string(), ());
                                            }
                                        }
                                    }
                                    "Glob" | "Grep" => {
                                        // These touch files but we don't count them as file ops
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                }
                "user" => {
                    total_user_turns += 1;
                    if !sid.is_empty() {
                        sessions.entry(sid.clone()).or_default().user_turns += 1;
                    }
                }
                _ => {}
            }
        }
    }

    // Build session stats
    let mut session_stats: Vec<SessionStats> = sessions
        .into_iter()
        .map(|(sid, acc)| {
            let duration_secs = match (acc.first_ts, acc.last_ts) {
                (Some(first), Some(last)) => (last - first).num_seconds().max(0) as u64,
                _ => 0,
            };
            let first_date = acc.first_ts.map(|dt| dt.date());
            SessionStats {
                session_id: sid,
                first_date,
                duration_secs,
                user_turns: acc.user_turns,
                assistant_turns: acc.assistant_turns,
                files_touched_count: acc.files_touched.len(),
            }
        })
        .collect();
    session_stats.sort_by(|a, b| b.duration_secs.cmp(&a.duration_secs));

    Metrics {
        daily_tokens,
        hour_counts,
        tool_calls,
        file_ops,
        file_edit_counts,
        sessions: session_stats,
        total_user_turns,
        total_assistant_turns,
        model_tokens,
        daily_model_tokens,
    }
}

/// Sum cost across all models for a given day's token totals.
pub fn estimate_daily_cost(day_models: &BTreeMap<String, ModelTokens>) -> f64 {
    day_models
        .iter()
        .map(|(model, tokens)| estimate_cost(model, tokens))
        .sum()
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn parse_date(ts: &str) -> Option<NaiveDate> {
    if ts.len() < 10 {
        return None;
    }
    NaiveDate::parse_from_str(&ts[..10], "%Y-%m-%d").ok()
}

fn parse_datetime(ts: &str) -> Option<NaiveDateTime> {
    // "2026-03-10T14:47:23.886Z"
    NaiveDateTime::parse_from_str(ts, "%Y-%m-%dT%H:%M:%S%.fZ")
        .or_else(|_| NaiveDateTime::parse_from_str(ts, "%Y-%m-%dT%H:%M:%SZ"))
        .ok()
}

use chrono::Timelike;

/// Shorten a file path for display: keep last 2 components
fn shorten_path(path: &str) -> &str {
    let bytes = path.as_bytes();
    let mut slashes = 0;
    for i in (0..bytes.len()).rev() {
        if bytes[i] == b'/' {
            slashes += 1;
            if slashes == 2 {
                return &path[i..];
            }
        }
    }
    path
}

/// Normalize model name to a canonical key for pricing lookup.
/// Maps aliases and versioned names to a short key.
fn normalize_model(model: &str) -> String {
    if model == "<synthetic>" || model.is_empty() {
        return String::new();
    }
    let m = model.to_lowercase();
    if m.contains("opus") && m.contains("4-6") {
        "Opus 4.6".to_string()
    } else if m.contains("opus") && m.contains("4-5") {
        "Opus 4.5".to_string()
    } else if m.contains("sonnet") && m.contains("4-5") {
        "Sonnet 4.5".to_string()
    } else if m.contains("haiku") && m.contains("4-5") {
        "Haiku 4.5".to_string()
    } else if m == "sonnet" {
        "Sonnet 4.5".to_string()
    } else if m == "haiku" {
        "Haiku 4.5".to_string()
    } else {
        model.to_string()
    }
}

/// Estimate cost in USD for a given model's token usage.
/// Prices are per million tokens from Anthropic's published pricing.
pub fn estimate_cost(model: &str, tokens: &ModelTokens) -> f64 {
    let (input_per_m, output_per_m, cache_write_per_m, cache_read_per_m) = match model {
        "Opus 4.6" | "Opus 4.5" => (5.0, 25.0, 6.25, 0.50),
        "Sonnet 4.5" => (3.0, 15.0, 3.75, 0.30),
        "Haiku 4.5" => (1.0, 5.0, 1.25, 0.10),
        _ => (5.0, 25.0, 6.25, 0.50), // default to Opus pricing
    };
    let m = 1_000_000.0;
    (tokens.input as f64 / m) * input_per_m
        + (tokens.output as f64 / m) * output_per_m
        + (tokens.cache_creation as f64 / m) * cache_write_per_m
        + (tokens.cache_read as f64 / m) * cache_read_per_m
}
