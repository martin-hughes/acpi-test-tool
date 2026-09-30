//! Method arguments, as typed after the method name in an `execute` command.
//!
//! An argument is either an integer or a string:
//!
//! * Integers are decimal, or hexadecimal with a leading `0x`: `42`, `0x2A`. They are unsigned
//!   and at most 64 bits.
//! * Strings are in double quotes, with Rust's escapes: `"hello"`, `"a \"quoted\" word"`,
//!   `"tab\there"`, `"\x41\u{42}"`. They can contain spaces.
//!
//! Anything else (hex without `0x`, unquoted text, negative numbers) is an error.

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Argument {
    Integer(u64),
    /// The characters are ASCII and never NUL, because AML strings are.
    String(String),
}

/// The message for calling a method with the wrong number of arguments. Every backend checks the
/// count itself (they each know how to find out what a method takes), and reports it this way.
pub fn wrong_argument_count(expected: usize, given: usize) -> String {
    let takes = match expected {
        0 => "takes no arguments".to_string(),
        1 => "takes 1 argument".to_string(),
        n => format!("takes {n} arguments"),
    };
    let were = if given == 1 { "was" } else { "were" };
    format!("the method {takes}, but {given} {were} given")
}

/// Parse the text that follows the method name. Empty text is no arguments.
pub fn parse_arguments(text: &str) -> Result<Vec<Argument>, String> {
    let mut arguments = Vec::new();
    let mut rest = text.trim_start();
    while !rest.is_empty() {
        let number = arguments.len() + 1;
        let (argument, remainder) =
            parse_one(rest).map_err(|message| format!("argument {number}: {message}"))?;
        arguments.push(argument);
        rest = remainder.trim_start();
    }
    Ok(arguments)
}

/// Parse one argument from the start of `text`, returning it and what follows it.
fn parse_one(text: &str) -> Result<(Argument, &str), String> {
    if let Some(after_quote) = text.strip_prefix('"') {
        let (string, remainder) = parse_string(after_quote)?;
        // "a"b would run two things together.
        if remainder.chars().next().is_some_and(|c| !c.is_whitespace()) {
            return Err("a string must be followed by a space".into());
        }
        return Ok((Argument::String(string), remainder));
    }

    let end = text.find(char::is_whitespace).unwrap_or(text.len());
    let (token, remainder) = text.split_at(end);
    Ok((Argument::Integer(parse_integer(token)?), remainder))
}

fn parse_integer(token: &str) -> Result<u64, String> {
    let invalid = || {
        format!(
            "\"{token}\" is not a valid argument (integers are decimal, or hexadecimal with a \
             leading 0x, and strings are in double quotes)"
        )
    };
    let (digits, radix) = match token.strip_prefix("0x") {
        Some(hex) => (hex, 16),
        None => (token, 10),
    };
    if digits.is_empty() || !digits.chars().all(|c| c.is_digit(radix)) {
        return Err(invalid());
    }
    u64::from_str_radix(digits, radix)
        .map_err(|_| format!("\"{token}\" is too large for a 64-bit integer"))
}

/// Parse a string's contents, after its opening quote, up to and including the closing quote.
fn parse_string(text: &str) -> Result<(String, &str), String> {
    let mut string = String::new();
    let mut chars = text.char_indices();
    while let Some((index, c)) = chars.next() {
        match c {
            '"' => {
                check_string(&string)?;
                return Ok((string, &text[index + 1..]));
            }
            '\\' => string.push(parse_escape(&mut chars)?),
            _ => string.push(c),
        }
    }
    Err("the string is missing its closing quote".into())
}

/// Parse an escape sequence, after its backslash.
fn parse_escape(chars: &mut std::str::CharIndices<'_>) -> Result<char, String> {
    let (_, c) = chars
        .next()
        .ok_or_else(|| "the string is missing its closing quote".to_string())?;
    Ok(match c {
        'n' => '\n',
        'r' => '\r',
        't' => '\t',
        '\\' => '\\',
        '0' => '\0',
        '\'' => '\'',
        '"' => '"',
        'x' => {
            let digits: String = chars.by_ref().take(2).map(|(_, c)| c).collect();
            let value = (digits.len() == 2)
                .then(|| u8::from_str_radix(&digits, 16).ok())
                .flatten()
                .filter(|_| digits.chars().all(|c| c.is_ascii_hexdigit()))
                .ok_or_else(|| "\\x must be followed by two hexadecimal digits".to_string())?;
            if value > 0x7F {
                return Err("\\x escapes must be at most \\x7F".into());
            }
            value as char
        }
        'u' => {
            if chars.next().map(|(_, c)| c) != Some('{') {
                return Err("\\u must be followed by {hexadecimal digits}".into());
            }
            let mut digits = String::new();
            loop {
                match chars.next().map(|(_, c)| c) {
                    Some('}') => break,
                    Some(c) if c.is_ascii_hexdigit() && digits.len() < 6 => digits.push(c),
                    _ => {
                        return Err("\\u must be followed by {up to six hexadecimal digits}".into());
                    }
                }
            }
            u32::from_str_radix(&digits, 16)
                .ok()
                .and_then(char::from_u32)
                .ok_or_else(|| format!("\\u{{{digits}}} is not a valid character"))?
        }
        other => return Err(format!("unknown escape \\{other}")),
    })
}

/// AML strings are ASCII, and end at the first NUL, so neither NUL nor anything past ASCII can be
/// passed to a method.
fn check_string(string: &str) -> Result<(), String> {
    if string.chars().any(|c| c == '\0' || !c.is_ascii()) {
        return Err("strings must be ASCII and cannot contain NUL".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Result<Vec<Argument>, String> {
        parse_arguments(text)
    }

    fn int(value: u64) -> Argument {
        Argument::Integer(value)
    }

    fn string(text: &str) -> Argument {
        Argument::String(text.into())
    }

    fn error(text: &str) -> String {
        parse(text).expect_err(text)
    }

    #[test]
    fn no_arguments() {
        assert_eq!(parse(""), Ok(vec![]));
        assert_eq!(parse("   \t "), Ok(vec![]));
    }

    #[test]
    fn decimal_integers() {
        assert_eq!(parse("0"), Ok(vec![int(0)]));
        assert_eq!(parse("42"), Ok(vec![int(42)]));
        assert_eq!(parse("007"), Ok(vec![int(7)]));
        assert_eq!(parse("18446744073709551615"), Ok(vec![int(u64::MAX)]));
    }

    #[test]
    fn hexadecimal_integers_need_a_0x_prefix() {
        assert_eq!(parse("0x0"), Ok(vec![int(0)]));
        assert_eq!(parse("0x2a"), Ok(vec![int(42)]));
        assert_eq!(parse("0x2A"), Ok(vec![int(42)]));
        assert_eq!(parse("0xFFFFFFFFFFFFFFFF"), Ok(vec![int(u64::MAX)]));
        assert!(error("ff").contains("not a valid argument"));
        assert!(error("2a").contains("not a valid argument"));
        assert!(error("0X2A").contains("not a valid argument"));
        assert!(error("x2a").contains("not a valid argument"));
    }

    #[test]
    fn malformed_integers() {
        for bad in [
            "0x", "0xg", "-1", "+1", "1.5", "12abc", "1_000", "0b101", "0x-1",
        ] {
            assert!(error(bad).contains("not a valid argument"), "{bad}");
        }
        assert!(error("18446744073709551616").contains("too large"));
        assert!(error("0x10000000000000000").contains("too large"));
    }

    #[test]
    fn plain_strings() {
        assert_eq!(parse(r#""hello""#), Ok(vec![string("hello")]));
        assert_eq!(parse(r#""""#), Ok(vec![string("")]));
        assert_eq!(parse(r#""two words""#), Ok(vec![string("two words")]));
        assert_eq!(parse(r#""  padded  ""#), Ok(vec![string("  padded  ")]));
    }

    #[test]
    fn strings_use_rust_escapes() {
        assert_eq!(parse(r#""a\nb""#), Ok(vec![string("a\nb")]));
        assert_eq!(parse(r#""a\tb\r""#), Ok(vec![string("a\tb\r")]));
        assert_eq!(parse(r#""back\\slash""#), Ok(vec![string("back\\slash")]));
        assert_eq!(parse(r#""say \"hi\"""#), Ok(vec![string("say \"hi\"")]));
        assert_eq!(parse(r#""it\'s""#), Ok(vec![string("it's")]));
        assert_eq!(parse(r#""\x41\x7f""#), Ok(vec![string("A\x7f")]));
        assert_eq!(parse(r#""\u{41}\u{0042}""#), Ok(vec![string("AB")]));
    }

    #[test]
    fn bad_escapes() {
        assert!(error(r#""\q""#).contains("unknown escape"));
        assert!(error(r#""\x""#).contains("two hexadecimal digits"));
        assert!(error(r#""\x4""#).contains("two hexadecimal digits"));
        assert!(error(r#""\xZZ""#).contains("two hexadecimal digits"));
        assert!(error(r#""\x80""#).contains("at most"));
        assert!(error(r#""\u41""#).contains("\\u must be followed"));
        assert!(error(r#""\u{}""#).contains("not a valid character"));
        assert!(error(r#""\u{110000}""#).contains("not a valid character"));
        assert!(error(r#""\u{D800}""#).contains("not a valid character"));
        assert!(error(r#""\u{1234567}""#).contains("up to six"));
    }

    #[test]
    fn strings_must_be_terminated() {
        assert!(error(r#""abc"#).contains("closing quote"));
        assert!(error(r#"""#).contains("closing quote"));
        // An escaped quote doesn't end the string.
        assert!(error(r#""abc\""#).contains("closing quote"));
        // Nor does a backslash with nothing after it.
        assert!(error(r#""abc\"#).contains("closing quote"));
    }

    #[test]
    fn strings_must_be_ascii_and_not_contain_nul() {
        assert!(error(r#""caf\u{e9}""#).contains("ASCII"));
        assert!(error("\"caf\u{e9}\"").contains("ASCII"));
        assert!(error(r#""a\0b""#).contains("NUL"));
        assert!(error(r#""\u{0}""#).contains("NUL"));
    }

    #[test]
    fn a_string_must_be_followed_by_a_space() {
        assert!(error(r#""a"b"#).contains("followed by a space"));
        assert!(error(r#""a""b""#).contains("followed by a space"));
        assert!(error(r#""a"1"#).contains("followed by a space"));
    }

    #[test]
    fn unquoted_text_is_not_a_string() {
        assert!(error("hello").contains("not a valid argument"));
        assert!(error(r#"hel"lo""#).contains("not a valid argument"));
        assert!(error("'a'").contains("not a valid argument"));
    }

    #[test]
    fn several_arguments_of_both_kinds() {
        assert_eq!(
            parse(r#"1 0x10 "two words" 3 "x""#),
            Ok(vec![
                int(1),
                int(16),
                string("two words"),
                int(3),
                string("x")
            ])
        );
        assert_eq!(parse("  1\t 2   "), Ok(vec![int(1), int(2)]));
        assert_eq!(parse(r#""a" "b""#), Ok(vec![string("a"), string("b")]));
    }

    #[test]
    fn errors_say_which_argument_is_at_fault() {
        assert!(error(r#"1 2 nope"#).starts_with("argument 3: "));
        assert!(error(r#"1 "unterminated"#).starts_with("argument 2: "));
        assert!(error("bad").starts_with("argument 1: "));
    }

    #[test]
    fn argument_count_messages() {
        assert_eq!(
            wrong_argument_count(0, 1),
            "the method takes no arguments, but 1 was given"
        );
        assert_eq!(
            wrong_argument_count(1, 0),
            "the method takes 1 argument, but 0 were given"
        );
        assert_eq!(
            wrong_argument_count(2, 3),
            "the method takes 2 arguments, but 3 were given"
        );
    }
}
