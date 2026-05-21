//! Question-answering for the live call. When a participant addresses
//! the bot by name in channel chat, we feed the rolling transcript +
//! their question to a Groq chat model and get back a short answer
//! suitable for both posting and speaking aloud.

use anyhow::{Context, Result};
use serde::Deserialize;

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    message: ChoiceMessage,
}

#[derive(Deserialize)]
struct ChoiceMessage {
    #[serde(default)]
    content: String,
}

const SYSTEM: &str = "You are utopia, an AI agent sitting in a live voice \
call. A participant has addressed you by name. Answer their question \
using the call transcript provided as context. Rules: answer in 1-3 \
short sentences — your reply will be spoken aloud, so keep it brief and \
conversational. Don't use markdown, bullet points, or emoji. If the \
transcript doesn't contain the answer, say so plainly. Don't invent \
facts. Don't repeat the question back.";

/// Answer `question` against `transcript` via Groq chat completions.
/// `transcript` is the joined `<nick>: <utterance>` lines so far (may
/// be empty early in a call).
pub async fn answer(
    client: &reqwest::Client,
    api_key: &str,
    model: &str,
    transcript: &str,
    question: &str,
) -> Result<String> {
    let context = if transcript.trim().is_empty() {
        "(no transcript yet — the call just started)".to_string()
    } else {
        transcript.to_string()
    };
    let user = format!("Call transcript so far:\n{context}\n\nQuestion: {question}");

    let body = serde_json::json!({
        "model": model,
        "max_tokens": 320,
        "temperature": 0.3,
        "messages": [
            { "role": "system", "content": SYSTEM },
            { "role": "user", "content": user },
        ],
    });

    let resp = client
        .post("https://api.groq.com/openai/v1/chat/completions")
        .bearer_auth(api_key)
        .json(&body)
        .send()
        .await
        .context("groq chat request failed")?;

    let status = resp.status();
    if !status.is_success() {
        let err = resp.text().await.unwrap_or_default();
        anyhow::bail!("groq chat {status}: {err}");
    }
    let parsed: ChatResponse = resp.json().await.context("groq chat parse failed")?;
    let text = parsed
        .choices
        .first()
        .map(|c| c.message.content.trim().to_string())
        .unwrap_or_default();
    if text.is_empty() {
        anyhow::bail!("groq chat returned no content");
    }
    Ok(text)
}

const SCENE_SYSTEM: &str = "You are utopia, an AI agent on a video call. \
You keep a live visual 'board' on your video tile. After each answer you \
update the board. Output ONLY a JSON object:\n\
{\"title\": \"short board title\", \"steps\": [\"short point\", ...]}\n\
Rules:\n\
- title: <= 5 words.\n\
- steps: 0 to 6 items, each <= 8 words, punchy, no trailing punctuation, \
plain text (no markdown, no emoji).\n\
- You are given the CURRENT board. Keep the points still worth showing \
— repeat them VERBATIM and in the same order at the FRONT of the list — \
then append new points drawn from the latest answer. The board \
accumulates across the call.\n\
- If the latest answer adds nothing worth showing, return the current \
board unchanged.";

/// Ask the model to evolve utopia's visual board for the latest answer.
/// `board` is the board's current points (carried forward + appended).
/// Returns `(title, steps)`, or `None` when there's nothing to show or
/// on any error — utopia then keeps its current tile. Never fails the
/// caller.
pub async fn generate_scene(
    client: &reqwest::Client,
    api_key: &str,
    model: &str,
    question: &str,
    answer: &str,
    board: &[String],
) -> Option<(String, Vec<String>)> {
    let board_str = if board.is_empty() {
        "(empty)".to_string()
    } else {
        board
            .iter()
            .map(|s| format!("- {s}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let user = format!(
        "Current board:\n{board_str}\n\nLatest question: {question}\n\
         Latest answer: {answer}\n\nUpdated board JSON:"
    );
    let body = serde_json::json!({
        "model": model,
        "max_tokens": 500,
        "temperature": 0.3,
        "response_format": { "type": "json_object" },
        "messages": [
            { "role": "system", "content": SCENE_SYSTEM },
            { "role": "user", "content": user },
        ],
    });
    let resp = client
        .post("https://api.groq.com/openai/v1/chat/completions")
        .bearer_auth(api_key)
        .json(&body)
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let parsed: ChatResponse = resp.json().await.ok()?;
    let text = parsed.choices.first()?.message.content.trim().to_string();
    let json = extract_json(&text)?;
    let title = json
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let steps: Vec<String> = json
        .get("steps")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default();
    if title.is_empty() && steps.is_empty() {
        return None;
    }
    Some((title, steps))
}

/// Pull a JSON object out of a model reply — it may be fenced in
/// markdown or wrapped in stray prose. Takes the outermost `{ … }`.
pub(crate) fn extract_json(text: &str) -> Option<serde_json::Value> {
    let start = text.find('{')?;
    let end = text.rfind('}')?.checked_add(1)?;
    if end <= start {
        return None;
    }
    serde_json::from_str(&text[start..end]).ok()
}

/// If `text` addresses `nick` at the start (`nick:`, `nick,`,
/// `@nick `, or bare `nick ` followed by words), return the remainder
/// — the actual question. Case-insensitive. `None` if the message
/// isn't addressed to the bot.
pub fn extract_addressed<'a>(text: &'a str, nick: &str) -> Option<&'a str> {
    let trimmed = text.trim_start();
    let lower = trimmed.to_lowercase();
    let nick_lower = nick.to_lowercase();

    for prefix in [
        format!("@{nick_lower} "),
        format!("{nick_lower}: "),
        format!("{nick_lower}, "),
        format!("{nick_lower} "),
    ] {
        if lower.starts_with(&prefix) {
            let rest = trimmed[prefix.len()..].trim();
            if !rest.is_empty() {
                return Some(rest);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_colon_form() {
        assert_eq!(
            extract_addressed("utopia: what did we decide?", "utopia"),
            Some("what did we decide?")
        );
    }

    #[test]
    fn extracts_comma_and_at_and_bare_forms() {
        assert_eq!(
            extract_addressed("utopia, summarize", "utopia"),
            Some("summarize")
        );
        assert_eq!(
            extract_addressed("@utopia who is talking", "utopia"),
            Some("who is talking")
        );
        assert_eq!(
            extract_addressed("utopia recap please", "utopia"),
            Some("recap please")
        );
    }

    #[test]
    fn case_insensitive_on_nick() {
        assert_eq!(
            extract_addressed("Utopia: hi", "utopia"),
            Some("hi")
        );
    }

    #[test]
    fn ignores_unaddressed_or_mid_sentence_mentions() {
        assert_eq!(extract_addressed("hello everyone", "utopia"), None);
        assert_eq!(
            extract_addressed("ask the utopia later", "utopia"),
            None,
            "a mention mid-sentence is not an address"
        );
    }

    #[test]
    fn ignores_bare_nick_with_no_question() {
        // Just the nick, nothing after → not a question.
        assert_eq!(extract_addressed("utopia", "utopia"), None);
        assert_eq!(extract_addressed("utopia: ", "utopia"), None);
    }

    #[test]
    fn extract_json_pulls_object_out_of_messy_replies() {
        // Bare object.
        let v = extract_json(r#"{"title":"T","steps":["a"]}"#).unwrap();
        assert_eq!(v["title"], "T");
        // Markdown-fenced with surrounding prose.
        let v = extract_json("Sure:\n```json\n{\"title\":\"T\"}\n```\nok").unwrap();
        assert_eq!(v["title"], "T");
        // No object, or invalid JSON → None, never a panic.
        assert!(extract_json("no json here").is_none());
        assert!(extract_json("{not valid").is_none());
    }
}
