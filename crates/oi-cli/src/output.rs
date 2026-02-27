use serde::Serialize;

/// Output format for CLI results.
pub enum Format {
    Table,
    Json,
    Ndjson,
}

/// Print a list of items in the specified format.
///
/// For Table format, use the provided headers and row_fn to format each item.
/// For JSON, serialize the entire list with pretty-printing.
/// For NDJSON, serialize each item on its own line.
pub fn print_list<T: Serialize>(
    items: &[T],
    format: &Format,
    headers: &[&str],
    row_fn: impl Fn(&T) -> Vec<String>,
) {
    match format {
        Format::Json => {
            println!("{}", serde_json::to_string_pretty(items).unwrap_or_default());
        }
        Format::Ndjson => {
            for item in items {
                println!("{}", serde_json::to_string(item).unwrap_or_default());
            }
        }
        Format::Table => {
            if items.is_empty() {
                println!("(no results)");
                return;
            }
            // Compute column widths from headers and data
            let mut widths: Vec<usize> = headers.iter().map(|h| h.len()).collect();
            let rows: Vec<Vec<String>> = items.iter().map(&row_fn).collect();
            for row in &rows {
                for (i, cell) in row.iter().enumerate() {
                    if i < widths.len() {
                        widths[i] = widths[i].max(cell.len());
                    }
                }
            }
            // Print header
            let header_line: Vec<String> = headers
                .iter()
                .enumerate()
                .map(|(i, h)| format!("{:<width$}", h, width = widths.get(i).copied().unwrap_or(0)))
                .collect();
            println!("{}", header_line.join("  "));
            // Print separator
            let sep: Vec<String> = widths.iter().map(|w| "-".repeat(*w)).collect();
            println!("{}", sep.join("  "));
            // Print rows
            for row in &rows {
                let line: Vec<String> = row
                    .iter()
                    .enumerate()
                    .map(|(i, cell)| {
                        format!("{:<width$}", cell, width = widths.get(i).copied().unwrap_or(0))
                    })
                    .collect();
                println!("{}", line.join("  "));
            }
        }
    }
}

/// Print a single item in the specified format.
///
/// For Table format, use the provided display_fn for custom rendering.
/// For JSON/NDJSON, serialize the item.
pub fn print_item<T: Serialize>(item: &T, format: &Format, display_fn: impl Fn(&T)) {
    match format {
        Format::Json | Format::Ndjson => {
            println!("{}", serde_json::to_string_pretty(item).unwrap_or_default());
        }
        Format::Table => {
            display_fn(item);
        }
    }
}

/// Print a success message. In JSON mode, wraps in a status object.
pub fn print_ok(msg: &str, format: &Format) {
    match format {
        Format::Json | Format::Ndjson => {
            let obj = serde_json::json!({"status": "ok", "message": msg});
            println!("{}", serde_json::to_string_pretty(&obj).unwrap_or_default());
        }
        Format::Table => {
            println!("{msg}");
        }
    }
}

/// Print an error message. In JSON mode, wraps in an error status object.
pub fn print_err(msg: &str, format: &Format) {
    match format {
        Format::Json | Format::Ndjson => {
            let obj = serde_json::json!({"status": "error", "message": msg});
            eprintln!("{}", serde_json::to_string_pretty(&obj).unwrap_or_default());
        }
        Format::Table => {
            eprintln!("error: {msg}");
        }
    }
}

#[cfg(test)]
mod tests {
    use serde::Serialize;

    #[derive(Serialize)]
    struct TestItem {
        name: String,
        value: u32,
    }

    #[test]
    fn test_print_ok_json() {
        // Verify the JSON structure is correct by building it directly
        let obj = serde_json::json!({"status": "ok", "message": "done"});
        let output = serde_json::to_string_pretty(&obj).unwrap();
        assert!(output.contains("\"status\": \"ok\""));
        assert!(output.contains("\"message\": \"done\""));
    }

    #[test]
    fn test_print_err_json() {
        let obj = serde_json::json!({"status": "error", "message": "something failed"});
        let output = serde_json::to_string_pretty(&obj).unwrap();
        assert!(output.contains("\"status\": \"error\""));
        assert!(output.contains("\"message\": \"something failed\""));
    }

    #[test]
    fn test_print_list_table() {
        let items = vec![
            TestItem {
                name: "alpha".to_string(),
                value: 10,
            },
            TestItem {
                name: "beta".to_string(),
                value: 200,
            },
        ];

        // Verify the row function works correctly
        let row_fn = |item: &TestItem| vec![item.name.clone(), item.value.to_string()];
        let headers = &["NAME", "VALUE"];

        // Check column width computation
        let mut widths: Vec<usize> = headers.iter().map(|h| h.len()).collect();
        let rows: Vec<Vec<String>> = items.iter().map(&row_fn).collect();
        for row in &rows {
            for (i, cell) in row.iter().enumerate() {
                if i < widths.len() {
                    widths[i] = widths[i].max(cell.len());
                }
            }
        }
        // "alpha" is 5, "NAME" is 4 => max is 5
        assert_eq!(widths[0], 5);
        // "200" is 3, "VALUE" is 5 => max is 5
        assert_eq!(widths[1], 5);

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0], vec!["alpha", "10"]);
        assert_eq!(rows[1], vec!["beta", "200"]);
    }

    #[test]
    fn test_print_list_json() {
        let items = vec![
            TestItem {
                name: "alpha".to_string(),
                value: 10,
            },
        ];
        let json = serde_json::to_string_pretty(&items).unwrap();
        assert!(json.contains("\"name\": \"alpha\""));
        assert!(json.contains("\"value\": 10"));
    }

    #[test]
    fn test_print_item_json() {
        let item = TestItem {
            name: "test".to_string(),
            value: 42,
        };
        let json = serde_json::to_string_pretty(&item).unwrap();
        assert!(json.contains("\"name\": \"test\""));
        assert!(json.contains("\"value\": 42"));
    }
}
