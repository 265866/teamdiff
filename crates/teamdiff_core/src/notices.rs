//! The third-party license notices `teamdiff --licenses` prints.
//!
//! The release build gets the notices as the HTML page `about.hbs` writes, which every release
//! archive of a trellis project ships. teamdiff has always printed them as plain text, laid out as
//! below, so this turns that page back into the text:
//!
//! ```text
//! teamdiff third-party licenses
//!
//! teamdiff is built with the open-source crates below. Each license text lists the crates it covers.
//!
//!   MIT License (2)
//!
//! ================================================================================
//! MIT License
//!
//! Used by:
//!   anyhow 1.0.104 (https://github.com/dtolnay/anyhow)
//!
//! <the license text>
//! ```
//!
//! From its `<h1>` heading to the end, anything on the page outside that layout is an error rather
//! than text dropped without a word, so a change to `about.hbs` stops the release build instead of
//! shipping wrong notices. Only the page's `<head>`, which the text doesn't show, goes unread.

use std::str::Split;

use anyhow::{anyhow, bail};

const TITLE: &str = "teamdiff third-party licenses";
const INTRO: &str = "teamdiff is built with the open-source crates below. Each license text lists the \
                     crates it covers.";
const RULE: &str =
    "================================================================================";

/// Turns the HTML notices page that `about.hbs` writes into the plain-text notices.
///
/// # Errors
///
/// Fails, naming the line, when the page isn't laid out as `about.hbs` writes it.
pub fn text_from_html(html: &str) -> anyhow::Result<String> {
    use std::fmt::Write as _;

    use anyhow::Context as _;

    let mut lines = Lines {
        rest: html.split('\n'),
        number: 0,
    };
    let mut text = format!("{TITLE}\n\n{INTRO}\n\n");
    lines.skip_to("<h1>Third-party licenses</h1>")?;
    lines.expect(&format!("<p>{INTRO}</p>"))?;
    lines.expect("<ul>")?;
    loop {
        let line = lines.next()?;
        if line.text == "</ul>" {
            break;
        }
        let item = enclosed(line.text, "  <li>", "</li>").with_context(|| at(line.number))?;
        writeln!(text, "  {}", unescape(item, line.number)?)?;
    }
    loop {
        let line = lines.next()?;
        if line.text == "</body>" {
            break;
        }
        let name = enclosed(line.text, "<h2>", "</h2>").with_context(|| at(line.number))?;
        write!(
            text,
            "\n{RULE}\n{}\n\nUsed by:\n",
            unescape(name, line.number)?
        )?;
        lines.expect("<p>Used by:</p>")?;
        lines.expect("<ul>")?;
        loop {
            let user = lines.next()?;
            if user.text == "</ul>" {
                break;
            }
            let anchor = enclosed(user.text, "  <li><a href=\"", "</a></li>")
                .with_context(|| at(user.number))?;
            let (url, crate_name) = anchor
                .split_once("\">")
                .ok_or_else(|| anyhow!("{} holds no link", at(user.number)))?;
            let crate_name = unescape(crate_name, user.number)?;
            writeln!(text, "  {crate_name} ({})", unescape(url, user.number)?)?;
        }
        let license = lines.pre()?;
        write!(text, "\n{}\n", unescape(&license.body, license.start)?)?;
    }
    lines.expect("</html>")?;
    lines.end()?;
    Ok(text)
}

/// A license text, with the number of the line it starts on.
struct Pre {
    start: usize,
    body: String,
}

/// One line of the page, with its number from 1, which errors name.
struct Line<'page> {
    number: usize,
    text: &'page str,
}

/// The page's lines, read one at a time.
struct Lines<'page> {
    rest: Split<'page, char>,
    number: usize,
}

impl<'page> Lines<'page> {
    fn next(&mut self) -> anyhow::Result<Line<'page>> {
        let text = self
            .rest
            .next()
            .ok_or_else(|| anyhow!("the license notices end too early"))?;
        self.number += 1;
        Ok(Line {
            number: self.number,
            text,
        })
    }

    fn expect(&mut self, wanted: &str) -> anyhow::Result<()> {
        let line = self.next()?;
        if line.text != wanted {
            bail!("{} should be {wanted:?}", at(line.number));
        }
        Ok(())
    }

    fn skip_to(&mut self, wanted: &str) -> anyhow::Result<()> {
        loop {
            let Some(text) = self.rest.next() else {
                bail!("the license notices have no line {wanted:?}");
            };
            self.number += 1;
            if text == wanted {
                return Ok(());
            }
        }
    }

    /// Checks nothing follows: the page ends with the newline after `</html>`.
    fn end(&mut self) -> anyhow::Result<()> {
        if self.rest.next() != Some("") || self.rest.next().is_some() {
            bail!("{} follows </html>", at(self.number + 1));
        }
        Ok(())
    }

    /// The text of a `<pre>` element, which may span lines, starting on the next line.
    fn pre(&mut self) -> anyhow::Result<Pre> {
        let first = self.next()?;
        let mut body = first
            .text
            .strip_prefix("<pre>")
            .ok_or_else(|| {
                anyhow!(
                    "{} should start a license text with <pre>",
                    at(first.number)
                )
            })?
            .to_owned();
        loop {
            if let Some(done) = body.strip_suffix("</pre>") {
                return Ok(Pre {
                    start: first.number,
                    body: done.to_owned(),
                });
            }
            let Some(line) = self.rest.next() else {
                bail!(
                    "the license text from {} never closes with </pre>",
                    at(first.number)
                );
            };
            self.number += 1;
            body.push('\n');
            body.push_str(line);
        }
    }
}

/// The part of `line` between `start` and `end`, when it is exactly that.
fn enclosed<'a>(line: &'a str, start: &str, end: &str) -> anyhow::Result<&'a str> {
    line.strip_prefix(start)
        .and_then(|rest| rest.strip_suffix(end))
        .ok_or_else(|| anyhow!("{line:?} should look like {start}...{end}"))
}

fn at(number: usize) -> String {
    format!("line {number} of the license notices")
}

/// Undoes the escaping Handlebars applies to `{{...}}`: `&`, `<`, `>`, `"`, `'`, `` ` `` and `=`.
/// Handlebars writes no other `&`, so any other one means the page wasn't escaped the way this
/// reads it, and is an error naming `number`, the line the text starts on.
fn unescape(text: &str, number: usize) -> anyhow::Result<String> {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        rest = &rest[amp..];
        let entity = [
            ("&amp;", '&'),
            ("&lt;", '<'),
            ("&gt;", '>'),
            ("&quot;", '"'),
            ("&#x27;", '\''),
            ("&#x60;", '`'),
            ("&#x3D;", '='),
        ]
        .into_iter()
        .find(|(name, _)| rest.starts_with(name));
        let Some((name, character)) = entity else {
            bail!("{} holds an & that Handlebars didn't write", at(number));
        };
        out.push(character);
        rest = &rest[name.len()..];
    }
    out.push_str(rest);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A page as `about.hbs` writes it, for two licenses.
    const PAGE: &str = "<!DOCTYPE html>
<html lang=\"en\">
<head>
<meta charset=\"utf-8\">
<title>teamdiff third-party licenses</title>
</head>
<body>
<h1>Third-party licenses</h1>
<p>teamdiff is built with the open-source crates below. Each license text lists the crates it covers.</p>
<ul>
  <li>MIT License (2)</li>
  <li>BSD 3-Clause &quot;New&quot; or &quot;Revised&quot; License (1)</li>
</ul>
<h2>MIT License</h2>
<p>Used by:</p>
<ul>
  <li><a href=\"https://github.com/example/one\">one 1.0.0</a></li>
  <li><a href=\"https://crates.io/crates/two\">two 0.2.0</a></li>
</ul>
<pre>Copyright &lt;year&gt; &amp; the authors

Permission is hereby granted, &#x27;as is&#x27;.
</pre>
<h2>BSD 3-Clause &quot;New&quot; or &quot;Revised&quot; License</h2>
<p>Used by:</p>
<ul>
  <li><a href=\"https://github.com/example/three?a&#x3D;1&amp;b&#x3D;2\">three 3.0.0</a></li>
</ul>
<pre>Redistribution is allowed.</pre>
</body>
</html>
";

    #[test]
    fn turns_the_page_into_the_text_layout() {
        let expected = "teamdiff third-party licenses

teamdiff is built with the open-source crates below. Each license text lists the crates it covers.

  MIT License (2)
  BSD 3-Clause \"New\" or \"Revised\" License (1)

================================================================================
MIT License

Used by:
  one 1.0.0 (https://github.com/example/one)
  two 0.2.0 (https://crates.io/crates/two)

Copyright <year> & the authors

Permission is hereby granted, 'as is'.


================================================================================
BSD 3-Clause \"New\" or \"Revised\" License

Used by:
  three 3.0.0 (https://github.com/example/three?a=1&b=2)

Redistribution is allowed.
";
        assert_eq!(
            text_from_html(PAGE).unwrap(),
            expected,
            "each part lands where the text layout puts it"
        );
    }

    #[test]
    fn keeps_carriage_returns_inside_a_license_text() {
        let page = PAGE.replace("Redistribution is allowed.", "line one\r\nline two\r\n");
        let text = text_from_html(&page).unwrap();
        assert!(
            text.ends_with("  three 3.0.0 (https://github.com/example/three?a=1&b=2)\n\nline one\r\nline two\r\n\n"),
            "a license text keeps its own line endings: {text:?}"
        );
    }

    #[test]
    fn refuses_a_page_in_another_layout() {
        let page = PAGE.replace("<p>Used by:</p>", "<p>Crates:</p>");
        let error = format!("{:#}", text_from_html(&page).unwrap_err());
        assert!(
            error.contains("line 15 of the license notices should be \"<p>Used by:</p>\""),
            "the error names the line and what it should hold: {error}"
        );
    }

    #[test]
    fn refuses_a_page_that_ends_early() {
        let page = &PAGE[..PAGE.find(" is allowed.").unwrap()];
        let error = format!("{:#}", text_from_html(page).unwrap_err());
        assert!(
            error.contains("the license text from line 29 of the license notices never closes"),
            "the error names where the open license text starts: {error}"
        );
    }

    #[test]
    fn refuses_an_ampersand_handlebars_did_not_write() {
        assert_eq!(
            unescape("&lt;a&gt; &amp;amp;", 3).unwrap(),
            "<a> &amp;",
            "each entity decodes once"
        );
        let error = format!("{:#}", unescape("a & b", 3).unwrap_err());
        assert!(
            error.contains("line 3 of the license notices holds an & that Handlebars didn't write"),
            "{error}"
        );
        let page = PAGE.replace("Redistribution is allowed.", "Rights & duties.");
        assert!(text_from_html(&page).is_err(), "an unescaped page fails");
    }

    #[test]
    fn refuses_a_reworded_intro_and_text_after_the_page() {
        let reworded = PAGE.replace("Each license text", "Every license text");
        let error = format!("{:#}", text_from_html(&reworded).unwrap_err());
        assert!(
            error.contains("line 9 of the license notices should be"),
            "{error}"
        );
        let trailing = format!("{PAGE}<script></script>\n");
        let error = format!("{:#}", text_from_html(&trailing).unwrap_err());
        assert!(error.contains("follows </html>"), "{error}");
    }

    #[test]
    fn names_a_missing_heading() {
        let page = PAGE.replace("<h1>Third-party licenses</h1>", "<h1>Licenses</h1>");
        let error = format!("{:#}", text_from_html(&page).unwrap_err());
        assert!(
            error.contains("have no line \"<h1>Third-party licenses</h1>\""),
            "{error}"
        );
    }
}
