use clipkeep_plugin_sdk::{Plugin, run_plugin};
use clipkeep_protocol::{ResultAction, ResultKind, SearchResult};
use std::{collections::BTreeMap, io};

const PLUGIN_ID: &str = "io.clipkeep.calculator";

fn main() -> io::Result<()> {
    run_plugin(CalculatorPlugin)
}

struct CalculatorPlugin;

impl Plugin for CalculatorPlugin {
    fn id(&self) -> &'static str {
        PLUGIN_ID
    }

    fn search(&mut self, query: &str, limit: usize) -> Result<Vec<SearchResult>, String> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let expression = query.trim().trim_start_matches('=').trim();
        if expression.is_empty() {
            return Ok(Vec::new());
        }
        let Ok(value) = evaluate(expression) else {
            return Ok(Vec::new());
        };
        if !value.is_finite() {
            return Ok(Vec::new());
        }
        let formatted = format_number(value);
        Ok(vec![SearchResult {
            plugin_id: PLUGIN_ID.into(),
            id: format!("calculator:{formatted}"),
            title: formatted.clone(),
            subtitle: expression.into(),
            preview: format!("{expression} = {formatted}"),
            kind: ResultKind::Command,
            score: 3_000.0,
            metadata: BTreeMap::from([
                ("expression".into(), expression.into()),
                ("result".into(), formatted),
                ("symbol_name".into(), "function".into()),
            ]),
            actions: vec![
                ResultAction {
                    id: "paste".into(),
                    title: "Paste Result".into(),
                    destructive: false,
                    default: true,
                    closes_window: true,
                    paste_to_previous: true,
                    input_fields: Vec::new(),
                },
                ResultAction {
                    id: "copy".into(),
                    title: "Copy Result".into(),
                    destructive: false,
                    default: false,
                    closes_window: false,
                    paste_to_previous: false,
                    input_fields: Vec::new(),
                },
            ],
        }])
    }

    fn execute(&mut self, result_id: &str, action_id: &str) -> Result<Option<String>, String> {
        if !matches!(action_id, "copy" | "paste") {
            return Err(format!("unknown calculator action: {action_id}"));
        }
        let value = result_id
            .strip_prefix("calculator:")
            .ok_or_else(|| "invalid calculator result".to_owned())?;
        arboard::Clipboard::new()
            .and_then(|mut clipboard| clipboard.set_text(value))
            .map_err(|error| error.to_string())?;
        Ok(Some("Result copied".into()))
    }
}

fn format_number(value: f64) -> String {
    if value == 0.0 {
        return "0".into();
    }
    if value.abs() >= 1e12 || value.abs() < 1e-9 {
        return format!("{value:.10e}").replace("e+", "e");
    }
    let value = format!("{value:.12}");
    value.trim_end_matches('0').trim_end_matches('.').to_owned()
}

fn evaluate(expression: &str) -> Result<f64, String> {
    let normalized = expression
        .replace('×', "*")
        .replace('÷', "/")
        .replace('−', "-");
    let mut parser = Parser {
        input: normalized.as_bytes(),
        position: 0,
    };
    let value = parser.expression()?;
    parser.whitespace();
    if parser.position != parser.input.len() {
        return Err("unexpected input".into());
    }
    Ok(value)
}

struct Parser<'a> {
    input: &'a [u8],
    position: usize,
}

impl Parser<'_> {
    fn expression(&mut self) -> Result<f64, String> {
        let mut value = self.term()?;
        loop {
            self.whitespace();
            if self.consume(b'+') {
                value += self.term()?;
            } else if self.consume(b'-') {
                value -= self.term()?;
            } else {
                return Ok(value);
            }
        }
    }

    fn term(&mut self) -> Result<f64, String> {
        let mut value = self.power()?;
        loop {
            self.whitespace();
            if self.consume(b'*') {
                value *= self.power()?;
            } else if self.consume(b'/') {
                let divisor = self.power()?;
                if divisor == 0.0 {
                    return Err("division by zero".into());
                }
                value /= divisor;
            } else if self.consume(b'%') {
                let divisor = self.power()?;
                if divisor == 0.0 {
                    return Err("division by zero".into());
                }
                value %= divisor;
            } else {
                return Ok(value);
            }
        }
    }

    fn power(&mut self) -> Result<f64, String> {
        let value = self.unary()?;
        self.whitespace();
        if self.consume(b'^') {
            Ok(value.powf(self.power()?))
        } else {
            Ok(value)
        }
    }

    fn unary(&mut self) -> Result<f64, String> {
        self.whitespace();
        if self.consume(b'+') {
            self.unary()
        } else if self.consume(b'-') {
            Ok(-self.unary()?)
        } else {
            self.primary()
        }
    }

    fn primary(&mut self) -> Result<f64, String> {
        self.whitespace();
        if self.consume(b'(') {
            let value = self.expression()?;
            self.whitespace();
            if !self.consume(b')') {
                return Err("missing closing parenthesis".into());
            }
            return Ok(value);
        }
        if self.peek().is_some_and(|byte| byte.is_ascii_alphabetic()) {
            let name = self.identifier();
            return match name.as_str() {
                "pi" => Ok(std::f64::consts::PI),
                "e" => Ok(std::f64::consts::E),
                _ => {
                    self.whitespace();
                    if !self.consume(b'(') {
                        return Err("function requires parentheses".into());
                    }
                    let value = self.expression()?;
                    self.whitespace();
                    if !self.consume(b')') {
                        return Err("missing closing parenthesis".into());
                    }
                    match name.as_str() {
                        "sqrt" if value >= 0.0 => Ok(value.sqrt()),
                        "abs" => Ok(value.abs()),
                        "sin" => Ok(value.sin()),
                        "cos" => Ok(value.cos()),
                        "tan" => Ok(value.tan()),
                        "ln" if value > 0.0 => Ok(value.ln()),
                        "log" if value > 0.0 => Ok(value.log10()),
                        _ => Err(format!("unknown function or invalid argument: {name}")),
                    }
                }
            };
        }
        self.number()
    }

    fn number(&mut self) -> Result<f64, String> {
        self.whitespace();
        let start = self.position;
        while self
            .peek()
            .is_some_and(|byte| byte.is_ascii_digit() || matches!(byte, b'.' | b'e' | b'E'))
        {
            self.position += 1;
            if matches!(
                self.input.get(self.position.wrapping_sub(1)),
                Some(b'e' | b'E')
            ) && self.peek().is_some_and(|byte| matches!(byte, b'+' | b'-'))
            {
                self.position += 1;
            }
        }
        std::str::from_utf8(&self.input[start..self.position])
            .ok()
            .and_then(|value| value.parse().ok())
            .ok_or_else(|| "expected a number".into())
    }

    fn identifier(&mut self) -> String {
        let start = self.position;
        while self.peek().is_some_and(|byte| byte.is_ascii_alphabetic()) {
            self.position += 1;
        }
        String::from_utf8_lossy(&self.input[start..self.position]).to_lowercase()
    }

    fn whitespace(&mut self) {
        while self.peek().is_some_and(|byte| byte.is_ascii_whitespace()) {
            self.position += 1;
        }
    }

    fn consume(&mut self, expected: u8) -> bool {
        if self.peek() == Some(expected) {
            self.position += 1;
            true
        } else {
            false
        }
    }

    fn peek(&self) -> Option<u8> {
        self.input.get(self.position).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn respects_precedence_and_parentheses() {
        assert_eq!(evaluate("2 + 3 * 4").unwrap(), 14.0);
        assert_eq!(evaluate("(2 + 3) * 4").unwrap(), 20.0);
        assert_eq!(evaluate("2^3^2").unwrap(), 512.0);
    }

    #[test]
    fn supports_constants_and_functions() {
        assert!((evaluate("sqrt(81) + sin(pi / 2)").unwrap() - 10.0).abs() < 1e-10);
        assert_eq!(evaluate("abs(-4) + log(100)").unwrap(), 6.0);
        assert_eq!(evaluate("6 × 7 ÷ 2").unwrap(), 21.0);
    }

    #[test]
    fn rejects_invalid_expressions() {
        assert!(evaluate("1 / 0").is_err());
        assert!(evaluate("2 +").is_err());
        assert!(evaluate("unknown(1)").is_err());
    }
}
