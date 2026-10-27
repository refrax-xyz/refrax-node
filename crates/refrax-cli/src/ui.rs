//! Terminal output: a small palette, aligned columns, human sizes. Honors `NO_COLOR` and pipes.

use std::io::IsTerminal;
use std::sync::OnceLock;
use std::time::Duration;

fn color_on() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("NO_COLOR").is_none() && std::io::stdout().is_terminal())
}

fn paint(code: &str, s: &str) -> String {
    if color_on() {
        format!("\x1b[{code}m{s}\x1b[0m")
    } else {
        s.to_string()
    }
}

/// Accent: actions, ids, live values.
pub fn violet(s: &str) -> String {
    paint("38;5;141", s)
}
/// Verified, done, rebuilt.
pub fn mint(s: &str) -> String {
    paint("38;5;48", s)
}
/// Middle of the spectrum, used for URLs.
pub fn cyan(s: &str) -> String {
    paint("38;5;81", s)
}
/// Failures and slashes.
pub fn red(s: &str) -> String {
    paint("38;5;203", s)
}
/// Pending, warnings.
pub fn amber(s: &str) -> String {
    paint("38;5;221", s)
}
/// Secondary text.
pub fn dim(s: &str) -> String {
    paint("38;5;244", s)
}
/// Emphasis.
pub fn bold(s: &str) -> String {
    paint("1", s)
}

/// `◆ refrax <cmd> · details` header line.
pub fn header(cmd: &str, rest: &str) {
    println!();
    let rest = if rest.is_empty() {
        String::new()
    } else {
        format!("{} {}", dim("·"), dim(rest))
    };
    println!(
        "  {} {} {}",
        violet("◆"),
        bold(&format!("refrax {cmd}")),
        rest
    );
    println!();
}

/// A step line: label, text, timing aligned in a column.
pub fn step(label: &str, text: &str, took: Option<Duration>) {
    let line = format!("  {}  {}", dim(&format!("{label:<9}")), text);
    match took {
        Some(t) => {
            let pad = 66usize.saturating_sub(width(&line)).max(2);
            println!("{line}{}{}", " ".repeat(pad), dim(&format!("{:>6}", ms(t))));
        }
        None => println!("{line}"),
    }
}

/// A key/value line.
pub fn kv(key: &str, value: &str) {
    println!("  {}  {}", dim(&format!("{key:<9}")), value);
}

/// Thousands separators.
pub fn num(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// 2.4 MiB
pub fn bytes(n: u64) -> String {
    const U: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{n} B")
    } else {
        format!("{v:.1} {}", U[i])
    }
}

/// 12 ms / 1.4 s
pub fn ms(d: Duration) -> String {
    let m = d.as_secs_f64() * 1000.0;
    if m < 1000.0 {
        format!("{m:.0} ms")
    } else {
        format!("{:.1} s", m / 1000.0)
    }
}

/// Visible width, ignoring ANSI escapes.
pub fn width(s: &str) -> usize {
    let mut w = 0;
    let mut esc = false;
    for c in s.chars() {
        if esc {
            if c == 'm' {
                esc = false;
            }
        } else if c == '\x1b' {
            esc = true;
        } else {
            w += 1;
        }
    }
    w
}

/// Print rows as aligned columns, header dimmed.
pub fn table(head: &[&str], rows: &[Vec<String>]) {
    let cols = head.len();
    let mut w: Vec<usize> = head.iter().map(|h| h.len()).collect();
    for r in rows {
        for (i, c) in r.iter().enumerate().take(cols) {
            w[i] = w[i].max(width(c));
        }
    }
    let line = |cells: Vec<String>| {
        let mut out = String::from("  ");
        for (i, c) in cells.iter().enumerate() {
            out.push_str(c);
            if i + 1 < cells.len() {
                out.push_str(&" ".repeat(w[i] - width(c) + 2));
            }
        }
        println!("{}", out.trim_end());
    };
    line(head.iter().map(|h| dim(h)).collect());
    for r in rows {
        line(r.clone());
    }
}

/// `node-07` style label for the n-th node (1-based).
pub fn node_label(i: usize) -> String {
    format!("node-{i:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_and_sizes() {
        assert_eq!(num(0), "0");
        assert_eq!(num(2_500), "2,500");
        assert_eq!(num(291_402_118), "291,402,118");
        assert_eq!(bytes(512), "512 B");
        assert_eq!(bytes(2_500_000), "2.4 MiB");
        assert_eq!(ms(Duration::from_millis(1400)), "1.4 s");
    }

    #[test]
    fn width_ignores_ansi() {
        assert_eq!(width("\x1b[38;5;141mabc\x1b[0m"), 3);
    }
}
