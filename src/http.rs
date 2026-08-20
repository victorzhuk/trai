use std::io::Read;

// Well above any legitimate verbose_json or chat-completion reply; a
// hostile or broken server streaming an endless body must not exhaust
// memory before the JSON parser ever rejects it.
pub const MAX_RESPONSE_BODY_BYTES: u64 = 16 * 1024 * 1024;

pub fn read_body(response: reqwest::blocking::Response) -> std::io::Result<String> {
    let mut body = String::new();
    response
        .take(MAX_RESPONSE_BODY_BYTES)
        .read_to_string(&mut body)?;
    Ok(body)
}
