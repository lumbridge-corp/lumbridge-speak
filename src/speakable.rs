//! Making text speakable: written forms become the words a person would say.
//!
//! Magpie has no text normalizer on our server. Measured by speaking a sentence and
//! transcribing it again (2026-11-10): plain prose comes back word for word in all five
//! voices, while digits, money, clock times, versions and paths come back as noise
//! ("PR 73 merged at 4:05 PM and cost $0.19" was heard as "PR merged Adad and Costin").
//! The same things written as words are heard correctly, so the front writes them as
//! words before the model sees them.
//!
//! The rules are general English and developer writing. Names that belong to one
//! installation (how a product name is said, ids nobody reads aloud) come from the
//! settings: `[say]` and `drop`.

use std::sync::LazyLock;

use regex::{Captures, Regex};

macro_rules! rx {
    ($name:ident, $pattern:expr) => {
        static $name: LazyLock<Regex> = LazyLock::new(|| Regex::new($pattern).expect(stringify!($name)));
    };
}

// ── numbers as words ─────────────────────────────────────────────────────────

const ONES: [&str; 20] = [
    "zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten", "eleven", "twelve", "thirteen",
    "fourteen", "fifteen", "sixteen", "seventeen", "eighteen", "nineteen",
];
const TENS: [&str; 10] = ["", "", "twenty", "thirty", "forty", "fifty", "sixty", "seventy", "eighty", "ninety"];
const MONTHS: [&str; 12] = [
    "January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December",
];

/// 4307 → "four thousand three hundred seven".
pub fn cardinal(n: u64) -> String {
    if n < 20 {
        return ONES[n as usize].to_string();
    }
    if n < 100 {
        let tens = TENS[(n / 10) as usize];
        return if n % 10 == 0 { tens.to_string() } else { format!("{tens}-{}", ONES[(n % 10) as usize]) };
    }
    if n < 1000 {
        let head = format!("{} hundred", ONES[(n / 100) as usize]);
        return if n % 100 == 0 { head } else { format!("{head} {}", cardinal(n % 100)) };
    }
    for (scale, name) in
        [(1_000_000_000_000_000u64, "quadrillion"), (1_000_000_000_000, "trillion"), (1_000_000_000, "billion"), (1_000_000, "million"), (1000, "thousand")]
    {
        if n >= scale {
            let head = format!("{} {name}", cardinal(n / scale));
            return if n % scale == 0 { head } else { format!("{head} {}", cardinal(n % scale)) };
        }
    }
    unreachable!()
}

/// 21 → "twenty-first".
pub fn ordinal(n: u64) -> String {
    let words = cardinal(n);
    let cut = words.rfind([' ', '-']).map_or(0, |i| i + 1);
    let (head, last) = words.split_at(cut);
    let last = match last {
        "one" => "first".to_string(),
        "two" => "second".to_string(),
        "three" => "third".to_string(),
        "five" => "fifth".to_string(),
        "eight" => "eighth".to_string(),
        "nine" => "ninth".to_string(),
        "twelve" => "twelfth".to_string(),
        w if w.ends_with('y') => format!("{}ieth", &w[..w.len() - 1]),
        w => format!("{w}th"),
    };
    format!("{head}{last}")
}

/// "0142" → "oh one four two".
fn digits(s: &str, zero: &str) -> String {
    s.chars()
        .filter_map(|c| c.to_digit(10))
        .map(|d| if d == 0 { zero } else { ONES[d as usize] })
        .collect::<Vec<_>>()
        .join(" ")
}

/// A four-digit number the way it is read aloud: years, ports and model numbers go in
/// pairs ("twenty twenty-six", "eighty-eight forty-one", "fifty ninety"). A zero in the
/// third place is only paired in a year ("nineteen oh five"): heard back, "forty-three
/// oh seven" was taken for 43.07, so those are said in full.
fn pairs(n: u64) -> String {
    let (hi, lo) = (n / 100, n % 100);
    if n % 1000 == 0 {
        cardinal(n)
    } else if (2000..2010).contains(&n) {
        format!("two thousand {}", ONES[lo as usize])
    } else if lo == 0 {
        format!("{} hundred", cardinal(hi))
    } else if lo < 10 && (1100..2000).contains(&n) {
        format!("{} oh {}", cardinal(hi), ONES[lo as usize])
    } else if lo < 10 {
        cardinal(n)
    } else {
        format!("{} {}", cardinal(hi), cardinal(lo))
    }
}

/// A run of digits standing alone.
fn integer(s: &str) -> String {
    if s.len() > 1 && s.starts_with('0') {
        return digits(s, "oh");
    }
    match s.parse::<u64>() {
        Ok(n) if s.len() == 4 => pairs(n),
        Ok(n) if s.len() <= 15 => cardinal(n),
        _ => digits(s, "zero"),
    }
}

/// A quantity: "1,250" → "one thousand two hundred fifty", with `frac` "50" →
/// "... point five zero".
fn quantity(whole: &str, frac: Option<&str>) -> String {
    let whole: String = whole.chars().filter(char::is_ascii_digit).collect();
    let head = whole.parse::<u64>().map_or_else(|_| digits(&whole, "zero"), cardinal);
    match frac {
        Some(f) if !f.is_empty() => format!("{head} point {}", digits(f, "zero")),
        _ => head,
    }
}

fn is_one(whole: &str, frac: Option<&str>) -> bool {
    whole == "1" && frac.is_none_or(str::is_empty)
}

fn clock(hour: u64, minute: u64) -> String {
    match minute {
        0 => cardinal(hour),
        m if m < 10 => format!("{} oh {}", cardinal(hour), ONES[m as usize]),
        m => format!("{} {}", cardinal(hour), cardinal(m)),
    }
}

fn date(year: u64, month: u64, day: u64) -> Option<String> {
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) || !(1000..=2999).contains(&year) {
        return None;
    }
    Some(format!("{} {}, {}", MONTHS[(month - 1) as usize], ordinal(day), pairs(year)))
}

// ── the rules, in the order they run ─────────────────────────────────────────

rx!(PICTOGRAPHS, r"[\x{1F000}-\x{1FAFF}\x{2600}-\x{27BF}\x{2B00}-\x{2BFF}\x{FE0F}\x{200D}]");
rx!(ABOUT, r"[~≈]\s?(\d)");
rx!(URL, r"https?://(?:www\.)?([^\s/)>\]]+)(?:/[^\s)>\]]*)?");
rx!(EMAIL, r"\b[\w.+-]+@[\w-]+(?:\.[\w-]+)+\b");
rx!(PATH, r#"(^|[\s("'=\[`])((?:~|\.{1,2})?(?:/[\w.@+-]+)+/?)"#);
rx!(DOMAIN, r#"\b((?:[A-Za-z0-9-]+\.)+)(com|ai|io|dev|net|org|app|xyz|gov|edu|tv)\b(?:/[^\s)>\]"']*)?"#);
rx!(
    FILE,
    r"\b([\w-]+)\.(py|rs|js|ts|tsx|jsx|json|toml|yaml|yml|md|sh|swift|txt|html|css|wav|mp3|mp4|png|jpg|jpeg|gif|svg|pdf|plist|service|timer|log|csv|sql|go|cpp|zig|lock|conf|env|gguf|zip)\b"
);
rx!(HASH, r"\b[0-9a-f]{7,40}\b");
rx!(EG, r"\be\.g\.,?");
rx!(IE, r"\bi\.e\.,?");
rx!(ETC, r"\betc\.(\s+[A-Z]|\s*$)?");
rx!(VERSUS, r"\bvs\.?(\s)");
rx!(TITLE, r"\b(Dr|Mr|Mrs|Ms|Prof)\.(\s)");
rx!(WITHOUT, r"\bw/o\b");
rx!(WITH, r"\bw/\s?");
rx!(
    MONEY,
    r"([$€£])\s?(\d{1,3}(?:,\d{3})+|\d+)(?:\.(\d+))?(?:(K|k|M|B|T|bn)\b|\s(thousand|million|billion|trillion)\b)?"
);
rx!(RATE_TOKENS, r"\btok/s\b");
rx!(RATE_REQUESTS, r"\breq/s\b");
rx!(PER, r"([A-Za-z0-9])/(s|sec|second|min|minute|h|hr|hour|day|wk|week|mo|month|yr|year|req|user|seat|token)\b");
rx!(PERCENT, r"(\d{1,3}(?:,\d{3})+|\d+)(?:\.(\d+))?\s?%");
rx!(TIME_MERIDIEM, r"\b(\d{1,2}):(\d{2})(?::\d{2})?\s?([AaPp])(?:[Mm]\b|\.[Mm]\.)");
rx!(HOUR_MERIDIEM, r"\b(\d{1,2})\s?([AaPp])(?:[Mm]\b|\.[Mm]\.)");
rx!(TIME, r"\b(\d{1,2}):(\d{2})(?::\d{2})?\b");
rx!(DATE_ISO, r"\b(\d{4})-(\d{2})-(\d{2})\b");
rx!(DATE_US, r"\b(\d{1,2})/(\d{1,2})/(\d{4})\b");
rx!(PHONE, r"(?:\+?1[ .-])?\(?\b(\d{3})\)?[ .-](\d{3})[ .-](\d{4})\b");
rx!(IP, r"\b(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})\b");
rx!(VERSION, r"\bv(\d+(?:\.\d+)*)\b");
rx!(VERSION_BARE, r"\b\d+(?:\.\d+){2,}\b");
rx!(ALWAYS, r"\b24/7\b");
rx!(RATIO, r"\b(\d+)/(\d+)\b");
rx!(RANGE_DASH, r"(\d)\s?–\s?(\d)");
rx!(RANGE_HYPHEN, r"\b(\d+)-(\d+)\b");
rx!(PLUS_AFTER, r"(\d)\+(\s|$|[),.;:!?])");
rx!(ORDINAL, r"\b(\d{1,4})(?:st|nd|rd|th)\b");
rx!(HASH_NUMBER, r"#(\d+)");
rx!(
    UNIT,
    r"\b(\d{1,3}(?:,\d{3})+|\d+)(?:\.(\d+))?(?:\s?(GiB|GB|MiB|MB|KB|kB|TB|ms|kHz|MHz|GHz|Hz|Mbps|Gbps|kbps|fps|px|km|kg|mph|kWh|kW|mins|min|hrs|hr|secs|sec|tok|s)|(K|k|M|B|x|h|W))\b"
);
rx!(DURATION, r"\b(?:(\d+)h\s?)?(\d+)m(?:\s?(\d+)s)?\b");
rx!(TIMES, r"(\d)\s?×");
rx!(DECIMAL, r"\b(\d+)\.(\d+)\b");
rx!(GROUPED, r"\b\d{1,3}(?:,\d{3})+\b");
rx!(UNDERSCORE, r"([A-Za-z0-9])_+([A-Za-z0-9])");
rx!(LETTER_DIGIT, r"([A-Za-z])(\d)");
rx!(DIGIT_LETTER, r"(\d)([A-Za-z])");
rx!(INTEGER, r"\b\d+\b");
rx!(ARROW, r"\s*(?:→|->|=>|⇒)\s*");
rx!(SPACED_DASH, r"\s[—–]\s");
rx!(SLASH, r"(\w)/(\w)");
rx!(LONE_SLASH, r"(^|\s)/(\w)");
rx!(EMPTY_BRACKETS, r"\(\s*\)|\[\s*\]");
rx!(SPACES, r"[ \t]{2,}");
rx!(SPACE_BEFORE_MARK, r"[ \t]+([,.;:!?])");
rx!(DOUBLED_MARK, r"([,;:])(?:\s*[,;:])+");

fn sub(text: &str, re: &Regex, with: impl Fn(&Captures) -> String) -> String {
    re.replace_all(text, |c: &Captures| with(c)).into_owned()
}

fn group<'a>(c: &'a Captures, i: usize) -> Option<&'a str> {
    c.get(i).map(|m| m.as_str())
}

fn num(c: &Captures, i: usize) -> u64 {
    c[i].parse().unwrap_or(0)
}

/// A file or folder name as words: "speech_front.py" → "speech front dot py".
fn name_words(name: &str) -> String {
    let mut out = String::new();
    let chars: Vec<char> = name.chars().collect();
    for (i, &ch) in chars.iter().enumerate() {
        match ch {
            '-' | '_' | '+' | '@' => out.push(' '),
            '.' if i > 0 && i + 1 < chars.len() => out.push_str(" dot "),
            '.' => {}
            other => out.push(other),
        }
    }
    out
}

fn unit_words(unit: &str, one: bool) -> &'static str {
    let (single, many) = match unit {
        "GB" | "GiB" => ("gigabyte", "gigabytes"),
        "MB" | "MiB" => ("megabyte", "megabytes"),
        "KB" | "kB" => ("kilobyte", "kilobytes"),
        "TB" => ("terabyte", "terabytes"),
        "ms" => ("millisecond", "milliseconds"),
        "s" | "sec" | "secs" => ("second", "seconds"),
        "min" | "mins" => ("minute", "minutes"),
        "h" | "hr" | "hrs" => ("hour", "hours"),
        "Hz" => ("hertz", "hertz"),
        "kHz" => ("kilohertz", "kilohertz"),
        "MHz" => ("megahertz", "megahertz"),
        "GHz" => ("gigahertz", "gigahertz"),
        "Mbps" => ("megabit per second", "megabits per second"),
        "Gbps" => ("gigabit per second", "gigabits per second"),
        "kbps" => ("kilobit per second", "kilobits per second"),
        "fps" => ("frame per second", "frames per second"),
        "px" => ("pixel", "pixels"),
        "km" => ("kilometer", "kilometers"),
        "kg" => ("kilogram", "kilograms"),
        "mph" => ("mile per hour", "miles per hour"),
        "kWh" => ("kilowatt hour", "kilowatt hours"),
        "kW" => ("kilowatt", "kilowatts"),
        "W" => ("watt", "watts"),
        "tok" => ("token", "tokens"),
        "K" | "k" => ("K", "K"),
        "M" => ("million", "million"),
        "B" => ("billion", "billion"),
        "x" => ("times", "times"),
        _ => ("", ""),
    };
    if one {
        single
    } else {
        many
    }
}

fn per_words(unit: &str) -> &str {
    match unit {
        "s" | "sec" => "second",
        "min" => "minute",
        "h" | "hr" => "hour",
        "wk" => "week",
        "mo" => "month",
        "yr" => "year",
        "req" => "request",
        other => other,
    }
}

fn money(c: &Captures) -> String {
    let (unit, units, cent, cents) = match &c[1] {
        "€" => ("euro", "euros", "cent", "cents"),
        "£" => ("pound", "pounds", "penny", "pence"),
        _ => ("dollar", "dollars", "cent", "cents"),
    };
    let (whole, frac) = (&c[2], group(c, 3));
    let scale = match group(c, 4).or(group(c, 5)) {
        Some("K" | "k" | "thousand") => Some("thousand"),
        Some("M" | "million") => Some("million"),
        Some("B" | "bn" | "billion") => Some("billion"),
        Some("T" | "trillion") => Some("trillion"),
        _ => None,
    };
    if let Some(scale) = scale {
        return format!("{} {scale} {units}", quantity(whole, frac));
    }
    let amount: u64 = whole.chars().filter(char::is_ascii_digit).collect::<String>().parse().unwrap_or(0);
    match frac {
        Some(f) if f.len() == 2 => {
            let small: u64 = f.parse().unwrap_or(0);
            let small_words = format!("{} {}", cardinal(small), if small == 1 { cent } else { cents });
            match (amount, small) {
                (0, _) => small_words,
                (_, 0) => format!("{} {}", cardinal(amount), if amount == 1 { unit } else { units }),
                _ => format!("{} {} and {small_words}", cardinal(amount), if amount == 1 { unit } else { units }),
            }
        }
        Some(f) => format!("{} {units}", quantity(whole, Some(f))),
        None => format!("{} {}", cardinal(amount), if amount == 1 { unit } else { units }),
    }
}

/// Written forms → spoken words, with nothing from the settings.
pub fn speakable(text: &str) -> String {
    let mut t = PICTOGRAPHS.replace_all(text, "").into_owned();
    t = sub(&t, &ABOUT, |c| format!("about {}", &c[1]));

    // Addresses and names nobody spells out.
    t = sub(&t, &URL, |c| c[1].to_string());
    t = sub(&t, &EMAIL, |c| c[0].replace('@', " at ").replace('.', " dot "));
    t = sub(&t, &PATH, |c| {
        let path = c[2].trim_end_matches('.');
        let stop = &c[2][path.len()..];
        let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty() && !matches!(*p, "~" | "." | "..")).collect();
        let named = path.starts_with(['~', '.']) || parts.len() > 1;
        let last = name_words(parts.last().copied().unwrap_or(""));
        format!("{}{}{last}{stop}", &c[1], if named { "" } else { "slash " })
    });
    t = sub(&t, &DOMAIN, |c| format!("{} dot {}", c[1].trim_end_matches('.').replace('.', " dot "), &c[2]));
    t = sub(&t, &FILE, |c| format!("{} dot {}", name_words(&c[1]), &c[2]));
    t = sub(&t, &HASH, |c| {
        let s = &c[0];
        let mixed = s.bytes().any(|b| b.is_ascii_digit()) && s.bytes().any(|b| b.is_ascii_lowercase());
        if mixed { String::new() } else { s.to_string() }
    });

    // Abbreviations, before their full stops are taken for sentence ends.
    t = sub(&t, &EG, |_| "for example,".to_string());
    t = sub(&t, &IE, |_| "that is,".to_string());
    t = sub(&t, &ETC, |c| match group(c, 1) {
        Some(after) => format!("et cetera.{after}"),
        None => "et cetera".to_string(),
    });
    t = sub(&t, &VERSUS, |c| format!("versus{}", &c[1]));
    t = sub(&t, &TITLE, |c| {
        let word = match &c[1] {
            "Dr" => "Doctor",
            "Mr" => "Mister",
            "Mrs" => "Missus",
            "Ms" => "Miz",
            _ => "Professor",
        };
        format!("{word}{}", &c[2])
    });
    t = sub(&t, &WITHOUT, |_| "without".to_string());
    t = sub(&t, &WITH, |_| "with ".to_string());

    // Money, rates, percentages.
    t = sub(&t, &MONEY, money);
    t = sub(&t, &RATE_TOKENS, |_| "tokens per second".to_string());
    t = sub(&t, &RATE_REQUESTS, |_| "requests per second".to_string());
    t = sub(&t, &PER, |c| format!("{} per {}", &c[1], per_words(&c[2])));
    t = sub(&t, &PERCENT, |c| format!("{} percent", quantity(&c[1], group(c, 2))));

    // Clock times and dates.
    t = sub(&t, &TIME_MERIDIEM, |c| {
        let (h, m) = (num(c, 1), num(c, 2));
        if h > 12 || m > 59 {
            return c[0].to_string();
        }
        format!("{} {} M", clock(h, m), c[3].to_uppercase())
    });
    t = sub(&t, &HOUR_MERIDIEM, |c| {
        let h = num(c, 1);
        if h == 0 || h > 12 {
            return c[0].to_string();
        }
        format!("{} {} M", cardinal(h), c[2].to_uppercase())
    });
    t = sub(&t, &TIME, |c| {
        let (h, m) = (num(c, 1), num(c, 2));
        match (h, m) {
            (h, m) if h > 23 || m > 59 => c[0].to_string(),
            (h, 0) if h <= 12 => format!("{} o'clock", cardinal(h)),
            (h, 0) => format!("{} hundred", cardinal(h)),
            (h, m) => clock(h, m),
        }
    });
    t = sub(&t, &DATE_ISO, |c| date(num(c, 1), num(c, 2), num(c, 3)).unwrap_or_else(|| c[0].to_string()));
    t = sub(&t, &DATE_US, |c| date(num(c, 3), num(c, 1), num(c, 2)).unwrap_or_else(|| c[0].to_string()));

    // Numbers that are read digit by digit or part by part.
    // "zero", not "oh": "eight oh oh" runs together. Groups of digits with commas between were
    // heard complete in all five voices, repeated digits included (2026-11-10, literal transcriber).
    t = sub(&t, &PHONE, |c| format!("{}, {}, {}", digits(&c[1], "zero"), digits(&c[2], "zero"), digits(&c[3], "zero")));
    t = sub(&t, &IP, |c| (1..=4).map(|i| cardinal(num(c, i))).collect::<Vec<_>>().join(" dot "));
    let version = |parts: &str| parts.split('.').map(|p| p.parse().map_or_else(|_| digits(p, "zero"), cardinal)).collect::<Vec<_>>().join(" point ");
    t = sub(&t, &VERSION, |c| format!("version {}", version(&c[1])));
    t = sub(&t, &VERSION_BARE, |c| format!("version {}", version(&c[0])));
    t = sub(&t, &ALWAYS, |_| "twenty-four seven".to_string());
    t = sub(&t, &RATIO, |c| match (&c[1], &c[2]) {
        ("1", "2") => "one half".to_string(),
        (a, b) => format!("{a} out of {b}"),
    });
    t = sub(&t, &RANGE_DASH, |c| format!("{} to {}", &c[1], &c[2]));
    t = sub(&t, &RANGE_HYPHEN, |c| {
        let (a, b) = (&c[1], &c[2]);
        let padded = (a.len() > 1 && a.starts_with('0')) || (b.len() > 1 && b.starts_with('0'));
        let rising = a.parse::<u64>().ok().zip(b.parse::<u64>().ok()).is_some_and(|(x, y)| x < y);
        if rising && !padded { format!("{a} to {b}") } else { format!("{a} {b}") }
    });
    t = sub(&t, &PLUS_AFTER, |c| format!("{} plus{}", &c[1], &c[2]));
    t = sub(&t, &ORDINAL, |c| ordinal(num(c, 1)));
    t = sub(&t, &HASH_NUMBER, |c| format!("number {}", &c[1]));

    // Durations with minutes ("3m 12s", "1h 5m"); a lone "5m" may be metres and is left alone.
    t = sub(&t, &DURATION, |c| {
        if c.get(1).is_none() && c.get(3).is_none() {
            return c[0].to_string();
        }
        let part = |i: usize, one: &str, many: &str| {
            c.get(i).map(|m| {
                let n: u64 = m.as_str().parse().unwrap_or(0);
                format!("{} {}", cardinal(n), if n == 1 { one } else { many })
            })
        };
        [part(1, "hour", "hours"), part(2, "minute", "minutes"), part(3, "second", "seconds")]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" ")
    });
    // Quantities with a unit, then every number that is left.
    t = sub(&t, &UNIT, |c| {
        let (whole, frac) = (&c[1], group(c, 2));
        let unit = group(c, 3).or(group(c, 4)).unwrap_or("");
        format!("{} {}", quantity(whole, frac), unit_words(unit, is_one(whole, frac)))
    });
    t = sub(&t, &TIMES, |c| format!("{} times", &c[1]));
    t = sub(&t, &DECIMAL, |c| quantity(&c[1], Some(&c[2])));
    t = sub(&t, &GROUPED, |c| quantity(&c[0], None));
    for _ in 0..2 {
        t = sub(&t, &UNDERSCORE, |c| format!("{} {}", &c[1], &c[2]));
    }
    t = sub(&t, &LETTER_DIGIT, |c| format!("{} {}", &c[1], &c[2]));
    t = sub(&t, &DIGIT_LETTER, |c| format!("{} {}", &c[1], &c[2]));
    t = sub(&t, &INTEGER, |c| integer(&c[0]));

    // Symbols.
    t = ARROW.replace_all(&t, " to ").into_owned();
    t = SPACED_DASH.replace_all(&t, ", ").into_owned();
    for (symbol, words) in [
        ("—", ", "), ("&", " and "), ("≥", " at least "), ("≤", " at most "), ("×", " times "), ("°", " degrees "),
        (" + ", " plus "), (" = ", " equals "), (" < ", " less than "), (" > ", " more than "), ("@", " at "),
    ] {
        t = t.replace(symbol, words);
    }
    for _ in 0..2 {
        t = sub(&t, &SLASH, |c| format!("{} {}", &c[1], &c[2]));
    }
    t = sub(&t, &LONE_SLASH, |c| format!("{}slash {}", &c[1], &c[2]));

    // Tidy what the rules left behind.
    t = EMPTY_BRACKETS.replace_all(&t, "").into_owned();
    t = SPACES.replace_all(&t, " ").into_owned();
    t = SPACE_BEFORE_MARK.replace_all(&t, "${1}").into_owned();
    t = DOUBLED_MARK.replace_all(&t, "${1}").into_owned();
    t.trim().to_string()
}

/// [`speakable`] with an installation's own names: how to say them, and what to leave out.
#[derive(Default)]
pub struct Speakable {
    say: Vec<(Regex, String)>,
    drop: Vec<Regex>,
}

impl Speakable {
    /// `say` maps a written name to how it is said; names match as whole words, in the
    /// letter case given, longest first. `drop` is a list of patterns that are removed.
    pub fn new<'a>(say: impl IntoIterator<Item = (&'a String, &'a String)>, drop: &[String]) -> Result<Self, String> {
        let mut names: Vec<(&String, &String)> = say.into_iter().filter(|(written, _)| !written.is_empty()).collect();
        names.sort_by(|a, b| b.0.chars().count().cmp(&a.0.chars().count()).then(a.0.cmp(b.0)));
        let word = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
        let say = names
            .into_iter()
            .map(|(written, said)| {
                let pattern = format!(
                    "{}{}{}",
                    if word(written.chars().next()) { r"\b" } else { "" },
                    regex::escape(written),
                    if word(written.chars().last()) { r"\b" } else { "" }
                );
                Regex::new(&pattern).map(|re| (re, said.clone())).map_err(|e| format!("say {written:?}: {e}"))
            })
            .collect::<Result<_, _>>()?;
        let drop = drop.iter().map(|p| Regex::new(p).map_err(|e| format!("drop {p:?}: {e}"))).collect::<Result<_, _>>()?;
        Ok(Self { say, drop })
    }

    pub fn apply(&self, text: &str) -> String {
        let mut t = text.to_string();
        for (written, said) in &self.say {
            t = written.replace_all(&t, regex::NoExpand(said)).into_owned();
        }
        for pattern in &self.drop {
            t = pattern.replace_all(&t, "").into_owned();
        }
        speakable(&t)
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    fn check(cases: &[(&str, &str)]) {
        for (written, said) in cases {
            assert_eq!(speakable(written), *said, "speakable({written:?})");
        }
    }

    #[test]
    fn numbers_as_words() {
        assert_eq!(cardinal(0), "zero");
        assert_eq!(cardinal(48), "forty-eight");
        assert_eq!(cardinal(4307), "four thousand three hundred seven");
        assert_eq!(cardinal(1_000_000), "one million");
        assert_eq!(cardinal(2_500_030), "two million five hundred thousand thirty");
        assert_eq!(ordinal(1), "first");
        assert_eq!(ordinal(12), "twelfth");
        assert_eq!(ordinal(20), "twentieth");
        assert_eq!(ordinal(23), "twenty-third");
        assert_eq!(ordinal(100), "one hundredth");
    }

    #[test]
    fn plain_prose_is_left_alone() {
        check(&[
            ("Good morning. Three things finished overnight, and one of them needs you.", "Good morning. Three things finished overnight, and one of them needs you."),
            ("I merged the pull request (the small one) and deployed it.", "I merged the pull request (the small one) and deployed it."),
            ("", ""),
        ]);
    }

    #[test]
    fn money_percent_and_rates() {
        check(&[
            ("It cost $0.19.", "It cost nineteen cents."),
            ("It cost $5.", "It cost five dollars."),
            ("It cost $1.", "It cost one dollar."),
            ("paid $1,250.50 today", "paid one thousand two hundred fifty dollars and fifty cents today"),
            ("a $20K budget", "a twenty thousand dollars budget"),
            ("raised $1.5M", "raised one point five million dollars"),
            ("raised $2 billion", "raised two billion dollars"),
            ("$100/mo", "one hundred dollars per month"),
            ("€40 or £3.01", "forty euros or three pounds and one penny"),
            ("95% of tests", "ninety-five percent of tests"),
            ("up 0.5 %", "up zero point five percent"),
            ("48 tok/s", "forty-eight tokens per second"),
            ("60 km/h", "sixty kilometers per hour"),
        ]);
    }

    #[test]
    fn times_and_dates() {
        check(&[
            ("at 4:05 PM.", "at four oh five P M."),
            ("at 4:30pm", "at four thirty P M"),
            ("before 9am", "before nine A M"),
            ("at 10 a.m. sharp", "at ten A M sharp"),
            ("at 17:13", "at seventeen thirteen"),
            ("at 9:00", "at nine o'clock"),
            ("on 2026-11-10", "on November tenth, twenty twenty-six"),
            ("on 11/10/2026", "on November tenth, twenty twenty-six"),
            ("on 7/4/2005", "on July fourth, two thousand five"),
            ("the 3rd try, the 21st day", "the third try, the twenty-first day"),
        ]);
    }

    #[test]
    fn numbers_by_how_they_are_read() {
        check(&[
            ("PR 73 merged", "PR seventy-three merged"),
            ("issue #94", "issue number ninety-four"),
            ("in 2026", "in twenty twenty-six"),
            ("port 8803", "port eight thousand eight hundred three"),
            ("port 8041", "port eighty forty-one"),
            ("in 1905", "in nineteen oh five"),
            ("the RTX 5090", "the RTX fifty ninety"),
            ("2000 files", "two thousand files"),
            ("1500 lines", "fifteen hundred lines"),
            ("4,307 tests", "four thousand three hundred seven tests"),
            ("65536 bytes", "sixty-five thousand five hundred thirty-six bytes"),
            ("line 0142", "line oh one four two"),
            ("pi is 3.14", "pi is three point one four"),
            ("Call 415-555-0142.", "Call four one five, five five five, zero one four two."),
            ("Call 800-555-0199 now", "Call eight zero zero, five five five, zero one nine nine now"),
            ("Call (888) 222-1111 or +1 315.555.0137", "Call eight eight eight, two two two, one one one one or three one five, five five five, zero one three seven"),
            ("at 192.0.2.17", "at one hundred ninety-two dot zero dot two dot seventeen"),
            ("v0.3.1 is out", "version zero point three point one is out"),
            ("vLLM 0.33.0", "vLLM version zero point thirty-three point zero"),
            ("scene v13", "scene version thirteen"),
            ("7/7 passed", "seven out of seven passed"),
            ("1/2 done", "one half done"),
            ("on call 24/7", "on call twenty-four seven"),
            ("5-10 minutes", "five to ten minutes"),
            ("1–3 s", "one to three seconds"),
            ("on 11-09", "on eleven oh nine"),
            ("95+ teams", "ninety-five plus teams"),
            ("~40 s apart", "about forty seconds apart"),
        ]);
    }

    #[test]
    fn units() {
        check(&[
            ("32GB of VRAM", "thirty-two gigabytes of VRAM"),
            ("1 GB free", "one gigabyte free"),
            ("1.9 GB", "one point nine gigabytes"),
            ("645 ms", "six hundred forty-five milliseconds"),
            ("in 1.1s", "in one point one seconds"),
            ("128K context", "one hundred twenty-eight K context"),
            ("a 357M model", "a three hundred fifty-seven million model"),
            ("a 30B model", "a thirty billion model"),
            ("that's 2x faster", "that's two times faster"),
            ("10× the price", "ten times the price"),
            ("22050 Hz", "twenty-two thousand fifty hertz"),
            ("200Mbps", "two hundred megabits per second"),
            ("ran 24h", "ran twenty-four hours"),
            ("took 3m 12s", "took three minutes twelve seconds"),
            ("took 3m12s", "took three minutes twelve seconds"),
            ("up 1h 5m", "up one hour five minutes"),
            ("1h 1m 1s left", "one hour one minute one second left"),
        ]);
    }

    #[test]
    fn names_with_digits_in_them() {
        check(&[
            ("the Pixel 10a", "the Pixel ten a"),
            ("an mp3 file", "an mp three file"),
            ("NVFP4 weights", "NVFP four weights"),
            ("on x86_64", "on x eighty-six sixty-four"),
            ("node-1 is up", "node-one is up"),
            ("a W-2 job", "a W-two job"),
        ]);
    }

    #[test]
    fn addresses_paths_and_hashes() {
        check(&[
            ("see https://docs.example.com/api/files now", "see docs dot example dot com now"),
            ("live at example.org.", "live at example dot org."),
            ("mail me@example.com", "mail me at example dot com"),
            ("see ~/src/lumbridge-app.", "see lumbridge app."),
            ("edit /etc/hosts first", "edit hosts first"),
            ("in src/main.rs", "in src main dot rs"),
            ("open speech_front.py", "open speech front dot py"),
            ("classic at /classic", "classic at slash classic"),
            ("run `~/bin/deploy` now", "run `deploy` now"),
            ("Commit 615c341 fixed it", "Commit fixed it"),
            ("the deadbeef word and 1234567 stay", "the deadbeef word and one million two hundred thirty-four thousand five hundred sixty-seven stay"),
        ]);
    }

    #[test]
    fn abbreviations_and_symbols() {
        check(&[
            ("e.g. tomorrow", "for example, tomorrow"),
            ("i.e., never", "that is, never"),
            ("pens, paper, etc. Then stop.", "pens, paper, et cetera. Then stop."),
            ("Leo vs. Aria", "Leo versus Aria"),
            ("Dr. Smith", "Doctor Smith"),
            ("w/ vocab, w/o a mic", "with vocab, without a mic"),
            ("TTS → ASR", "TTS to ASR"),
            ("the UI/UX review", "the UI UX review"),
            ("R&D", "R and D"),
            ("done — next", "done, next"),
            ("shipped ✅ today 🚀", "shipped today"),
            ("a ≥ 3", "a at least three"),
        ]);
    }

    #[test]
    fn spoken_text_stays_as_it_is() {
        // What comes out goes in again unchanged: callers that already wrote words lose nothing.
        for written in ["PR 73 merged at 4:05 PM and cost $0.19.", "The model runs at 48 tok/s with 128K context.", "v0.3.1 on 2026-11-10, 95% (7/7)."] {
            let once = speakable(written);
            assert_eq!(speakable(&once), once, "second pass over {written:?}");
            assert!(!once.bytes().any(|b| b.is_ascii_digit()), "digits left in {once:?}");
        }
    }

    #[test]
    fn names_from_the_settings() {
        let say = [("NixOS".to_string(), "Nix O S".to_string()), ("QA".to_string(), "Q A".to_string()), ("C++".to_string(), "C plus plus".to_string())];
        let drop = [r"\bt[0-9][0-9a-z]{1,2}\b".to_string()];
        let s = Speakable::new(say.iter().map(|(a, b)| (a, b)), &drop).unwrap();
        assert_eq!(s.apply("NixOS runs on QA, not on QATAR or qa."), "Nix O S runs on Q A, not on QATAR or qa.");
        assert_eq!(s.apply("a C++ server"), "a C plus plus server");
        assert_eq!(s.apply("the loop in t3q is fixed"), "the loop in is fixed");
        assert!(Speakable::new(std::iter::empty(), &["(".to_string()]).is_err());
    }
}
