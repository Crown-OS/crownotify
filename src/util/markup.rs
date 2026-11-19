//! The freedesktop body-markup subset, reduced to the plain text a card shows.
//!
//! The daemon advertises `body-markup` so that senders escape a literal `<`
//! or `&`. Cards render plain text, so tags are dropped and entities decoded.

use std::borrow::Cow;

/// `body` with its markup tags removed and its entities decoded.
pub fn plain_text(body: &str) -> Cow<'_, str> {
    if !body.contains(['<', '&']) {
        return Cow::Borrowed(body);
    }

    let mut text = String::with_capacity(body.len());
    let mut rest = body;
    while let Some(position) = rest.find(['<', '&']) {
        text.push_str(&rest[..position]);
        rest = &rest[position..];
        rest = if rest.starts_with('<') {
            skip_tag(rest, &mut text)
        } else {
            decode_entity(rest, &mut text)
        };
    }
    text.push_str(rest);
    Cow::Owned(text)
}

fn skip_tag<'body>(rest: &'body str, text: &mut String) -> &'body str {
    let opens_tag = rest[1..]
        .chars()
        .next()
        .is_some_and(|next| next.is_ascii_alphabetic() || next == '/');
    match rest.find('>') {
        Some(end) if opens_tag => &rest[end + 1..],
        _ => {
            text.push('<');
            &rest[1..]
        }
    }
}

fn decode_entity<'body>(rest: &'body str, text: &mut String) -> &'body str {
    let decoded = rest
        .find(';')
        .filter(|&end| end <= 10)
        .and_then(|end| Some((entity_character(&rest[1..end])?, end)));
    match decoded {
        Some((character, end)) => {
            text.push(character);
            &rest[end + 1..]
        }
        None => {
            text.push('&');
            &rest[1..]
        }
    }
}

fn entity_character(name: &str) -> Option<char> {
    match name {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        "nbsp" => Some('\u{a0}'),
        _ => {
            let number = name.strip_prefix('#')?;
            let code = match number.strip_prefix(['x', 'X']) {
                Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                None => number.parse().ok()?,
            };
            char::from_u32(code)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_is_borrowed_unchanged() {
        assert!(matches!(
            plain_text("See you at 5"),
            Cow::Borrowed("See you at 5")
        ));
    }

    #[test]
    fn tags_are_dropped_and_their_text_kept() {
        assert_eq!(
            plain_text(
                "<b>Alice</b>: <a href=\"https://example.com\">link</a><img src=\"/tmp/x.png\" alt=\"\"/>"
            ),
            "Alice: link"
        );
    }

    #[test]
    fn entities_are_decoded() {
        assert_eq!(
            plain_text("Tom &amp; Jerry &lt;3 &quot;hi&quot; &#39;yo&#x27; &#128512;"),
            "Tom & Jerry <3 \"hi\" 'yo' 😀"
        );
    }

    #[test]
    fn stray_angle_brackets_and_ampersands_survive() {
        assert_eq!(
            plain_text("1 < 2 & 3 > 2 <3 &unknown; a&b"),
            "1 < 2 & 3 > 2 <3 &unknown; a&b"
        );
    }

    #[test]
    fn an_unterminated_tag_is_kept_as_text() {
        assert_eq!(plain_text("x <b y"), "x <b y");
    }
}
