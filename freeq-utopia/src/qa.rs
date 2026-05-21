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

const CARD_SYSTEM: &str = "You are utopia, an AI agent on a video call. \
Produce a visual-aid card to display on your video tile. Output a single \
SVG document and NOTHING else.\n\
Hard requirements:\n\
- Root: <svg xmlns=\"http://www.w3.org/2000/svg\" width=\"640\" height=\"360\" viewBox=\"0 0 640 360\">\n\
- First child: an opaque full-size <rect width=\"640\" height=\"360\"> in a \
dark fill (e.g. #0b1020).\n\
- Light text (#e6edff / #9fb4e8), font-family=\"Helvetica, Arial, sans-serif\".\n\
- Content: a short bold title near the top, then EITHER up to 4 concise \
bullet lines OR a simple labelled box-and-arrow diagram. Type >= 20px. \
Keep wide margins; do not let text overflow 640x360.\n\
- No external images, no <script>, no <foreignObject>, no CSS classes.\n\
- Output ONLY the raw SVG, starting with <svg and ending with </svg>. \
If a visual genuinely would not help, output exactly: NONE";

/// Ask the model for an SVG visual-aid card illustrating an answer.
/// Returns `None` when a visual wouldn't help, or on any error — utopia
/// then simply keeps showing its presence. Never fails the caller.
pub async fn generate_card(
    client: &reqwest::Client,
    api_key: &str,
    model: &str,
    question: &str,
    answer: &str,
) -> Option<String> {
    let body = serde_json::json!({
        "model": model,
        "max_tokens": 1400,
        "temperature": 0.4,
        "messages": [
            { "role": "system", "content": CARD_SYSTEM },
            { "role": "user", "content":
                format!("Question: {question}\nAnswer: {answer}\n\nVisual-aid SVG:") },
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
    extract_svg(&text)
}

/// Pull a `<svg>…</svg>` document out of a model reply — it may be
/// fenced in markdown or wrapped in stray prose.
pub(crate) fn extract_svg(text: &str) -> Option<String> {
    let start = text.find("<svg")?;
    let end = text.rfind("</svg>")?.checked_add("</svg>".len())?;
    if end <= start {
        return None;
    }
    Some(text[start..end].to_string())
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
    fn extract_svg_pulls_doc_out_of_messy_replies() {
        // Bare SVG.
        assert_eq!(
            extract_svg("<svg><rect/></svg>").as_deref(),
            Some("<svg><rect/></svg>")
        );
        // Markdown-fenced with surrounding prose.
        assert_eq!(
            extract_svg("Sure!\n```svg\n<svg a=\"1\"><rect/></svg>\n```\nDone.")
                .as_deref(),
            Some("<svg a=\"1\"><rect/></svg>")
        );
        // A model that declined → no SVG.
        assert_eq!(extract_svg("NONE"), None);
        assert_eq!(extract_svg("no visual needed here"), None);
        // Truncated (no closing tag) → None, not a panic.
        assert_eq!(extract_svg("<svg><rect/>"), None);
    }
}
