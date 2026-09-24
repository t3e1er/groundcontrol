//! Cypher-Lite ASCII pattern parser.

use std::collections::HashMap;

use groundcontrol_common::{Error, Result};

use super::ast::{EdgePattern, NodePattern, PathPattern, QueryDirection};

struct PatternParser<'a> {
    input: &'a str,
}

impl<'a> PatternParser<'a> {
    fn new(input: &'a str) -> Self {
        Self { input }
    }

    fn skip_ws(&mut self) {
        self.input = self.input.trim_start();
    }

    fn peek(&self) -> Option<char> {
        self.input.chars().next()
    }

    fn starts_with(&self, prefix: &str) -> bool {
        self.input.starts_with(prefix)
    }

    fn consume_str(&mut self, prefix: &str) -> bool {
        self.skip_ws();
        if self.input.starts_with(prefix) {
            self.input = &self.input[prefix.len()..];
            true
        } else {
            false
        }
    }

    fn expect_str(&mut self, prefix: &str) -> Result<()> {
        self.skip_ws();
        if self.consume_str(prefix) {
            Ok(())
        } else {
            Err(Error::Config(format!("expected '{}' at: '{}'", prefix, self.input)))
        }
    }

    fn parse_ident(&mut self) -> Option<String> {
        self.skip_ws();
        let mut len = 0;
        for c in self.input.chars() {
            if c.is_alphanumeric() || c == '_' || c == '.' || c == '-' {
                len += c.len_utf8();
            } else {
                break;
            }
        }
        if len == 0 {
            None
        } else {
            let s = &self.input[..len];
            self.input = &self.input[len..];
            Some(s.to_string())
        }
    }

    fn parse_string_literal(&mut self) -> Result<String> {
        self.skip_ws();
        let quote = match self.peek() {
            Some('\'') => '\'',
            Some('"') => '"',
            _ => {
                return Err(Error::Config(format!("expected string literal at: '{}'", self.input)))
            }
        };
        self.input = &self.input[quote.len_utf8()..];
        let mut len = 0;
        let mut found_close = false;
        for c in self.input.chars() {
            if c == quote {
                found_close = true;
                break;
            }
            len += c.len_utf8();
        }
        if !found_close {
            return Err(Error::Config("unterminated string literal".to_string()));
        }
        let s = self.input[..len].to_string();
        self.input = &self.input[len + quote.len_utf8()..];
        Ok(s)
    }

    fn parse_node(&mut self) -> Result<NodePattern> {
        self.expect_str("(")?;
        self.skip_ws();

        let mut variable = None;
        let mut label = None;

        if let Some(id1) = self.parse_ident() {
            if self.consume_str(":") {
                variable = Some(id1);
                if let Some(lbl) = self.parse_ident() {
                    label = Some(lbl);
                }
            } else {
                variable = Some(id1);
            }
        } else if self.consume_str(":") {
            if let Some(lbl) = self.parse_ident() {
                label = Some(lbl);
            }
        }

        let mut properties = HashMap::new();
        if self.consume_str("{") {
            loop {
                self.skip_ws();
                if self.consume_str("}") {
                    break;
                }
                let key = self.parse_ident().ok_or_else(|| {
                    Error::Config(format!("expected property key at: '{}'", self.input))
                })?;
                self.expect_str(":")?;
                self.skip_ws();
                let val = if self.peek() == Some('\'') || self.peek() == Some('"') {
                    self.parse_string_literal()?
                } else if let Some(val_id) = self.parse_ident() {
                    val_id
                } else {
                    return Err(Error::Config(format!(
                        "expected property value at: '{}'",
                        self.input
                    )));
                };
                properties.insert(key, val);
                self.skip_ws();
                if self.consume_str(",") {
                    continue;
                } else if self.consume_str("}") {
                    break;
                } else {
                    return Err(Error::Config(format!(
                        "expected ',' or '}}' in properties at: '{}'",
                        self.input
                    )));
                }
            }
        }

        self.expect_str(")")?;

        Ok(NodePattern { variable, label, properties })
    }

    fn parse_edge(&mut self) -> Result<EdgePattern> {
        self.skip_ws();

        // 1. Shorthand checks: -->, <--, --
        if self.consume_str("-->") {
            return Ok(EdgePattern {
                direction: QueryDirection::Outgoing,
                edge_types: Vec::new(),
                min_hops: 1,
                max_hops: 1,
            });
        }
        if self.consume_str("<--") {
            return Ok(EdgePattern {
                direction: QueryDirection::Incoming,
                edge_types: Vec::new(),
                min_hops: 1,
                max_hops: 1,
            });
        }
        if self.consume_str("--") {
            return Ok(EdgePattern {
                direction: QueryDirection::Undirected,
                edge_types: Vec::new(),
                min_hops: 1,
                max_hops: 1,
            });
        }

        // 2. Full edge syntax: (<- | -) [ (:types)? (*range)? ] (-> | -)
        let inbound = self.consume_str("<-");
        if !inbound {
            self.expect_str("-")?;
        }
        self.expect_str("[")?;

        let mut edge_types = Vec::new();
        if self.consume_str(":") {
            loop {
                if let Some(et) = self.parse_ident() {
                    edge_types.push(et);
                }
                self.skip_ws();
                if self.consume_str("|") {
                    continue;
                } else {
                    break;
                }
            }
        }

        let mut min_hops = 1;
        let mut max_hops = 1;

        if self.consume_str("*") {
            self.skip_ws();
            let mut num1_str = String::new();
            while let Some(c) = self.peek() {
                if c.is_ascii_digit() {
                    num1_str.push(c);
                    self.input = &self.input[1..];
                } else {
                    break;
                }
            }
            if !num1_str.is_empty() {
                min_hops = num1_str.parse::<usize>().unwrap_or(1);
            }
            if self.consume_str("..") {
                let mut num2_str = String::new();
                while let Some(c) = self.peek() {
                    if c.is_ascii_digit() {
                        num2_str.push(c);
                        self.input = &self.input[1..];
                    } else {
                        break;
                    }
                }
                max_hops =
                    if !num2_str.is_empty() { num2_str.parse::<usize>().unwrap_or(5) } else { 5 };
            } else {
                max_hops = min_hops;
            }
        }

        self.expect_str("]")?;

        let outbound = self.consume_str("->");
        if !outbound {
            self.expect_str("-")?;
        }

        let direction = match (inbound, outbound) {
            (true, false) => QueryDirection::Incoming,
            (false, true) => QueryDirection::Outgoing,
            _ => QueryDirection::Undirected,
        };

        Ok(EdgePattern { direction, edge_types, min_hops, max_hops: max_hops.max(min_hops) })
    }

    fn parse(&mut self) -> Result<PathPattern> {
        let start_node = self.parse_node()?;
        let mut steps = Vec::new();

        loop {
            self.skip_ws();
            if self.input.is_empty() {
                break;
            }
            if self.starts_with("-") || self.starts_with("<-") {
                let edge = self.parse_edge()?;
                let node = self.parse_node()?;
                steps.push((edge, node));
            } else {
                break;
            }
        }

        Ok(PathPattern { start_node, steps })
    }
}

/// Parse a linear Cypher-Lite path pattern from string.
pub fn parse_path_pattern(input: &str) -> Result<PathPattern> {
    let mut parser = PatternParser::new(input);
    let pattern = parser.parse()?;
    parser.skip_ws();
    if !parser.input.is_empty() {
        return Err(Error::Config(format!(
            "unexpected trailing characters in graph_match pattern: '{}'",
            parser.input
        )));
    }
    Ok(pattern)
}
