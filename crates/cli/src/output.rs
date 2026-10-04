//! Salida del CLI (ADR-015): los datos van a stdout (texto o `--json`) y el diagnóstico a
//! stderr. La forma del JSON es parte del contrato y está cubierta por snapshots.

use std::io::{IsTerminal, Read, Write};

use serde::Serialize;

use crate::error::{CliError, CliResult, exit};

#[derive(Debug, Clone, Copy)]
pub struct Out {
    pub json: bool,
    pub no_input: bool,
}

impl Out {
    /// Escribe el resultado: JSON o el texto que devuelve `human`.
    pub fn data<T: Serialize>(&self, value: &T, human: impl FnOnce() -> String) {
        let mut stdout = std::io::stdout().lock();
        if self.json {
            let text = serde_json::to_string_pretty(value).expect("los datos son serializables");
            let _ = writeln!(stdout, "{text}");
        } else {
            let text = human();
            if !text.is_empty() {
                let _ = writeln!(stdout, "{}", text.trim_end());
            }
        }
    }

    /// Mensaje informativo para personas: a stderr, también con `--json`.
    pub fn note(&self, message: impl AsRef<str>) {
        eprintln!("{}", message.as_ref());
    }

    pub fn error(&self, e: &CliError) {
        if self.json {
            let body = serde_json::json!({ "error": e });
            eprintln!(
                "{}",
                serde_json::to_string_pretty(&body).expect("serializable")
            );
            return;
        }
        eprintln!("error: {}: {}", e.code, e.message);
        if let Some(action) = &e.action {
            eprintln!("  acción: {action}");
        }
        if let Some(id) = &e.request_id {
            eprintln!("  request_id: {id}");
        }
    }

    /// Pide confirmación para una operación destructiva. Con `--yes` no pregunta; sin
    /// terminal o con `--no-input` exige `--yes`.
    pub fn confirm(&self, question: &str, yes: bool) -> CliResult<()> {
        if yes {
            return Ok(());
        }
        if self.no_input || !std::io::stdin().is_terminal() {
            return Err(CliError::new(
                exit::USAGE,
                "CONFIRMATION_REQUIRED",
                format!("{question}: se necesita confirmación"),
            )
            .with_action("repite el comando con --yes"));
        }
        eprint!("{question} [s/N]: ");
        let _ = std::io::stderr().flush();
        let mut answer = String::new();
        std::io::stdin()
            .read_line(&mut answer)
            .map_err(|e| CliError::io("stdin", &e))?;
        if matches!(
            answer.trim().to_lowercase().as_str(),
            "s" | "si" | "sí" | "y" | "yes"
        ) {
            Ok(())
        } else {
            Err(CliError::new(
                exit::ERROR,
                "CANCELLED",
                "operación cancelada",
            ))
        }
    }

    /// Lee un secreto: de stdin con `--token-stdin`, o del terminal sin eco.
    pub fn read_secret(&self, prompt: &str, from_stdin: bool) -> CliResult<String> {
        let secret = if from_stdin {
            let mut s = String::new();
            std::io::stdin()
                .read_to_string(&mut s)
                .map_err(|e| CliError::io("stdin", &e))?;
            s
        } else {
            if self.no_input || !std::io::stdin().is_terminal() {
                return Err(CliError::usage("no se puede pedir el token sin terminal")
                    .with_action("pásalo por stdin con --token-stdin"));
            }
            rpassword::prompt_password(prompt).map_err(|e| CliError::io("terminal", &e))?
        };
        let secret = secret.trim().to_owned();
        if secret.is_empty() {
            return Err(CliError::usage("el token está vacío"));
        }
        Ok(secret)
    }
}

/// Tabla de texto con columnas alineadas.
pub fn table(headers: &[&str], rows: Vec<Vec<String>>) -> String {
    if rows.is_empty() {
        return "(sin resultados)".to_owned();
    }
    let mut widths: Vec<usize> = headers.iter().map(|h| h.chars().count()).collect();
    for row in &rows {
        for (i, cell) in row.iter().enumerate() {
            widths[i] = widths[i].max(cell.chars().count());
        }
    }
    let line = |cells: Vec<String>| {
        cells
            .iter()
            .enumerate()
            .map(|(i, c)| format!("{c:<width$}", width = widths[i]))
            .collect::<Vec<_>>()
            .join("  ")
            .trim_end()
            .to_owned()
    };
    let mut out = vec![line(headers.iter().map(|h| (*h).to_owned()).collect())];
    out.extend(rows.into_iter().map(line));
    out.join("\n")
}

pub fn opt(value: &Option<impl ToString>) -> String {
    value.as_ref().map_or("-".to_owned(), ToString::to_string)
}

pub fn bytes(n: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KiB", "MiB", "GiB"];
    let mut value = n as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{n} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_align_columns() {
        let t = table(
            &["NAME", "ROLE"],
            vec![
                vec!["ci".into(), "publisher".into()],
                vec!["alice".into(), "reader".into()],
            ],
        );
        assert_eq!(t, "NAME   ROLE\nci     publisher\nalice  reader");
        assert_eq!(table(&["X"], vec![]), "(sin resultados)");
    }

    #[test]
    fn human_sizes() {
        assert_eq!(bytes(10), "10 B");
        assert_eq!(bytes(1536), "1.5 KiB");
        assert_eq!(bytes(5 << 20), "5.0 MiB");
    }
}
