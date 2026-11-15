//! Optional LaTeX-to-text plugin for XuanCi.
//!
//! Protocol (no files, no network):
//!   xuanci-latex.exe --version
//!   echo '<request-json>' | xuanci-latex.exe
//!
//! Request: `{ "tex": "...", "display": true }`
//! Response: `{ "ok": true, "segments": [{"text":"x","style":"normal"}, ...] }`
//!
//! This is a readable-text converter, not a typesetting engine. Fractions become
//! `(num)/(den)`, roots become `√(...)`, and scripts are returned as super/sub
//! segments so the host can format them natively in RichEdit.

use std::io::Read;

#[derive(serde::Deserialize)]
struct Request {
    tex: String,
    #[serde(default)]
    #[allow(dead_code)]
    display: bool,
}

#[derive(serde::Serialize)]
struct Segment {
    text: String,
    style: &'static str,
}

const VERSION: &str = "xuanci-latex 0.2.0";
const MAX_TEX_BYTES: usize = 20_000;
const MAX_SEGMENTS: usize = 4000;

fn main() {
    if std::env::args().any(|arg| arg == "--version") {
        println!("{VERSION}");
        return;
    }

    let mut input = String::new();
    if let Err(error) = std::io::stdin().read_to_string(&mut input) {
        fail(&format!("read stdin: {error}"));
        return;
    }
    let request: Request = match serde_json::from_str(&input) {
        Ok(request) => request,
        Err(error) => {
            fail(&format!("parse request: {error}"));
            return;
        }
    };
    if request.tex.trim().is_empty() {
        fail("empty formula");
        return;
    }
    if request.tex.len() > MAX_TEX_BYTES {
        fail("formula too long");
        return;
    }

    let segments = Converter::new(&request.tex).convert();
    if segments.is_empty() {
        fail("no readable output");
        return;
    }
    let response = serde_json::json!({ "ok": true, "segments": segments });
    println!("{response}");
}

fn fail(message: &str) {
    let json = serde_json::json!({ "ok": false, "error": message });
    println!("{json}");
    std::process::exit(1);
}

struct Converter {
    chars: Vec<char>,
    index: usize,
}

impl Converter {
    fn new(tex: &str) -> Self {
        Self {
            chars: tex.chars().collect(),
            index: 0,
        }
    }

    fn convert(mut self) -> Vec<Segment> {
        self.sequence(None, "normal")
    }

    fn sequence(&mut self, end: Option<char>, style: &'static str) -> Vec<Segment> {
        let mut out = Vec::new();
        while self.index < self.chars.len() {
            if out.len() >= MAX_SEGMENTS {
                break;
            }
            let ch = self.chars[self.index];
            if Some(ch) == end {
                self.index += 1;
                break;
            }
            match ch {
                '{' => {
                    self.index += 1;
                    out.extend(self.sequence(Some('}'), style));
                }
                '}' => {
                    self.index += 1;
                }
                '\\' => {
                    self.index += 1;
                    let command = self.command_name();
                    self.command(&command, style, &mut out);
                }
                '^' => {
                    self.index += 1;
                    out.extend(self.script(style, "super"));
                }
                '_' => {
                    self.index += 1;
                    out.extend(self.script(style, "sub"));
                }
                '&' => {
                    self.index += 1;
                    push_text(&mut out, " ", style);
                }
                _ => {
                    self.index += 1;
                    push_char(&mut out, ch, style);
                }
            }
        }
        out
    }

    fn command_name(&mut self) -> String {
        if self.index >= self.chars.len() {
            return String::new();
        }
        let first = self.chars[self.index];
        if first.is_ascii_alphabetic() {
            let start = self.index;
            while self.index < self.chars.len() && self.chars[self.index].is_ascii_alphabetic() {
                self.index += 1;
            }
            self.chars[start..self.index].iter().collect()
        } else {
            self.index += 1;
            first.to_string()
        }
    }

    fn script(&mut self, current: &'static str, requested: &'static str) -> Vec<Segment> {
        let target = if current == "normal" { requested } else { current };
        if self.index >= self.chars.len() {
            return Vec::new();
        }
        if self.chars[self.index] == '{' {
            self.index += 1;
            return self.sequence(Some('}'), target);
        }
        if self.chars[self.index] == '\\' {
            self.index += 1;
            let command = self.command_name();
            let mut out = Vec::new();
            self.command(&command, target, &mut out);
            return out;
        }
        let ch = self.chars[self.index];
        self.index += 1;
        vec![Segment {
            text: ch.to_string(),
            style: target,
        }]
    }

    fn argument(&mut self, style: &'static str) -> Vec<Segment> {
        self.script("normal", style)
    }

    fn command(&mut self, command: &str, style: &'static str, out: &mut Vec<Segment>) {
        match command {
            "frac" | "dfrac" | "tfrac" => {
                let numerator = self.argument(style);
                let denominator = self.argument(style);
                push_text(out, "(", style);
                out.extend(numerator);
                push_text(out, ")/(", style);
                out.extend(denominator);
                push_text(out, ")", style);
            }
            "sqrt" => {
                let mut exponent = Vec::new();
                if self.index < self.chars.len() && self.chars[self.index] == '[' {
                    self.index += 1;
                    exponent = self.sequence(Some(']'), "super");
                }
                let argument = self.argument(style);
                out.extend(exponent);
                push_text(out, "√(", style);
                out.extend(argument);
                push_text(out, ")", style);
            }
            "text" | "mathrm" | "mathbf" | "mathit" | "mathsf" | "mathtt"
            | "operatorname" => {
                out.extend(self.argument(style));
            }
            "overline" | "underline" | "widehat" | "hat" | "bar" | "vec" => {
                out.extend(self.argument(style));
            }
            "left" | "right" => {
                if self.index < self.chars.len() {
                    let delimiter = self.chars[self.index];
                    self.index += 1;
                    if delimiter != '.' {
                        push_char(out, delimiter, style);
                    }
                }
            }
            "begin" | "end" => {
                let _ = self.argument("normal");
                push_text(out, " ", style);
            }
            "quad" | "qquad" => push_text(out, "  ", style),
            "," | ";" | ":" | " " | "\\" => push_text(out, " ", style),
            "!" => {}
            "displaystyle" | "textstyle" | "scriptstyle" | "scriptscriptstyle" => {}
            _ => {
                if let Some(symbol) = symbol(command) {
                    push_text(out, symbol, style);
                } else if is_function(command) {
                    push_text(out, &format!("{command} "), style);
                } else {
                    push_text(out, command, style);
                }
            }
        }
    }
}

fn push_char(out: &mut Vec<Segment>, ch: char, style: &'static str) {
    out.push(Segment {
        text: ch.to_string(),
        style,
    });
}

fn push_text(out: &mut Vec<Segment>, text: &str, style: &'static str) {
    if out.len() >= MAX_SEGMENTS {
        return;
    }
    let last = out.last_mut();
    if let Some(last) = last {
        if last.style == style && last.text.len() + text.len() <= 64 {
            last.text.push_str(text);
            return;
        }
    }
    out.push(Segment {
        text: text.to_string(),
        style,
    });
}

fn symbol(command: &str) -> Option<&'static str> {
    Some(match command {
        "alpha" => "α", "beta" => "β", "gamma" => "γ", "delta" => "δ",
        "epsilon" | "varepsilon" => "ε", "zeta" => "ζ", "eta" => "η",
        "theta" | "vartheta" => "θ", "iota" => "ι", "kappa" => "κ",
        "lambda" => "λ", "mu" => "μ", "nu" => "ν", "xi" => "ξ",
        "pi" | "varpi" => "π", "rho" | "varrho" => "ρ", "sigma" | "varsigma" => "σ",
        "tau" => "τ", "upsilon" => "υ", "phi" | "varphi" => "φ", "chi" => "χ",
        "psi" => "ψ", "omega" => "ω",
        "Gamma" => "Γ", "Delta" => "Δ", "Theta" => "Θ", "Lambda" => "Λ",
        "Xi" => "Ξ", "Pi" => "Π", "Sigma" => "Σ", "Upsilon" => "Υ",
        "Phi" => "Φ", "Psi" => "Ψ", "Omega" => "Ω",
        "pm" => "±", "mp" => "∓", "times" => "×", "div" => "÷", "cdot" => "·",
        "ast" => "∗", "star" => "⋆", "circ" => "∘", "bullet" => "•",
        "le" | "leq" => "≤", "ge" | "geq" => "≥", "ne" | "neq" => "≠",
        "approx" => "≈", "equiv" => "≡", "sim" => "∼", "simeq" => "≃",
        "propto" => "∝", "ll" => "≪", "gg" => "≫",
        "infty" => "∞", "partial" => "∂", "nabla" => "∇",
        "sum" => "∑", "prod" => "∏", "coprod" => "∐",
        "int" => "∫", "iint" => "∬", "iiint" => "∭", "oint" => "∮",
        "rightarrow" | "to" => "→", "leftarrow" | "gets" => "←",
        "leftrightarrow" => "↔", "Rightarrow" => "⇒", "Leftarrow" => "⇐",
        "Leftrightarrow" => "⇔", "mapsto" => "↦",
        "in" => "∈", "notin" => "∉", "subset" => "⊂", "supset" => "⊃",
        "subseteq" => "⊆", "supseteq" => "⊇", "cup" => "∪", "cap" => "∩",
        "forall" => "∀", "exists" => "∃", "neg" => "¬", "land" => "∧", "lor" => "∨",
        "ldots" | "dots" => "…", "cdots" => "⋯", "vdots" => "⋮", "ddots" => "⋱",
        "angle" => "∠", "degree" => "°", "prime" => "′", "perp" => "⊥",
        "parallel" => "∥", "cong" => "≅", "emptyset" | "varnothing" => "∅",
        "{" => "{", "}" => "}", "%" => "%", "$" => "$", "_" => "_", "&" => "&",
        "#" => "#", "\\" => " ", "space" => " ",
        _ => return None,
    })
}

fn is_function(command: &str) -> bool {
    matches!(
        command,
        "sin" | "cos" | "tan" | "cot" | "sec" | "csc"
            | "arcsin" | "arccos" | "arctan"
            | "sinh" | "cosh" | "tanh"
            | "log" | "ln" | "lg" | "exp"
            | "lim" | "max" | "min" | "sup" | "inf"
            | "det" | "gcd" | "lcm" | "mod" | "bmod"
            | "arg" | "deg" | "dim" | "ker" | "hom"
    )
}
