use std::time::Duration;

use super::*;
use crate::db::{Column, Value};

fn sample() -> (Vec<Column>, Vec<Row>) {
    let cols = vec![
        Column::new("id", "int4"),
        Column::new("name", "text"),
        Column::new("note", "text"),
        Column::new("amount", "numeric"),
    ];
    let rows = vec![
        vec![Value::Int(1), Value::Text("alice".into()), Value::Null, Value::Text("12.50".into())],
        vec![Value::Int(22), Value::Text("日本語".into()), Value::Text("two\nlines".into()), Value::Text("3".into())],
    ];
    (cols, rows)
}

fn opts(format: TableFormat) -> OutputOptions {
    OutputOptions { format, ..OutputOptions::default() }
}

fn colored(format: TableFormat) -> OutputOptions {
    OutputOptions { format, color: ColorDepth::TrueColor, ..OutputOptions::default() }
}

fn text(s: &str) -> Value {
    Value::Text(s.into())
}

#[test]
fn rounded_golden_numbers_right_aligned_cjk_double_width_multiline_split() {
    let (c, r) = sample();
    let expected = "\
╭────┬────────┬───────┬────────╮
│ id │ name   │ note  │ amount │
├────┼────────┼───────┼────────┤
│  1 │ alice  │ NULL  │  12.50 │
│ 22 │ 日本語 │ two   │      3 │
│    │        │ lines │        │
╰────┴────────┴───────┴────────╯
";
    assert_eq!(render(&c, &r, &opts(TableFormat::Rounded)), expected);
}

#[test]
fn row_lines_separate_every_row_but_not_the_lines_of_one_multiline_cell() {
    let (c, r) = sample();
    let expected = "\
╭────┬────────┬───────┬────────╮
│ id │ name   │ note  │ amount │
├────┼────────┼───────┼────────┤
│  1 │ alice  │ NULL  │  12.50 │
├────┼────────┼───────┼────────┤
│ 22 │ 日本語 │ two   │      3 │
│    │        │ lines │        │
╰────┴────────┴───────┴────────╯
";
    let lines = OutputOptions { row_lines: true, ..opts(TableFormat::Rounded) };
    assert_eq!(render(&c, &r, &lines), expected);
    let psql = OutputOptions { row_lines: true, ..opts(TableFormat::Psql) };
    assert_eq!(render(&c, &r, &psql), render(&c, &r, &opts(TableFormat::Psql)), "psql has no box to draw rules in");
}

#[test]
fn psql_golden_centered_header_no_trailing_spaces() {
    let (c, r) = sample();
    let expected = concat!(
        " id |  name  | note  | amount\n",
        "----+--------+-------+--------\n",
        "  1 | alice  | NULL  |  12.50\n",
        " 22 | 日本語 | two   |      3\n",
        "    |        | lines |\n",
    );
    assert_eq!(render(&c, &r, &opts(TableFormat::Psql)), expected);
}

#[test]
fn ascii_golden_matches_mysql_client_look() {
    let (c, r) = sample();
    let expected = "\
+----+--------+-------+--------+
| id | name   | note  | amount |
+----+--------+-------+--------+
|  1 | alice  | NULL  |  12.50 |
| 22 | 日本語 | two   |      3 |
|    |        | lines |        |
+----+--------+-------+--------+
";
    assert_eq!(render(&c, &r, &opts(TableFormat::Ascii)), expected);
}

#[test]
fn simple_minimal_plain_goldens() {
    let (c, r) = sample();
    assert_eq!(
        render(&c, &r, &opts(TableFormat::Simple)),
        "id  name    note   amount\n--  ------  -----  ------\n 1  alice   NULL    12.50\n22  日本語  two         3\n            lines\n"
    );
    assert_eq!(
        render(&c, &r, &opts(TableFormat::Minimal)),
        "id  name    note   amount\n──  ──────  ─────  ──────\n 1  alice   NULL    12.50\n22  日本語  two         3\n            lines\n"
    );
    assert_eq!(
        render(&c, &r, &opts(TableFormat::Plain)),
        "id  name    note   amount\n 1  alice   NULL    12.50\n22  日本語  two         3\n            lines\n"
    );
}

#[test]
fn every_bordered_format_has_equal_width_lines() {
    let (c, mut r) = sample();
    r.push(vec![Value::Int(3), text("😀 emoji"), text("tab\there"), Value::Float(1.5)]);
    for f in [TableFormat::Rounded, TableFormat::Unicode, TableFormat::Double, TableFormat::Ascii, TableFormat::Markdown] {
        let out = render(&c, &r, &opts(f));
        let widths: Vec<usize> = out.lines().map(table::str_width).collect();
        assert!(widths.windows(2).all(|w| w[0] == w[1]), "{f:?} misaligned:\n{out}");
    }
}

#[test]
fn markdown_escapes_pipes_and_marks_numeric_alignment() {
    let cols = vec![Column::new("n", "int"), Column::new("s", "text")];
    let rows = vec![vec![Value::Int(7), text("a|b\nc")]];
    let expected = "| n | s         |\n|--:|-----------|\n| 7 | a\\|b<br>c |\n";
    assert_eq!(render(&cols, &rows, &opts(TableFormat::Markdown)), expected);
}

#[test]
fn csv_quotes_per_rfc4180_and_null_is_empty() {
    let cols = vec![Column::new("a", "text"), Column::new("b", "text")];
    let rows = vec![vec![text("x,y"), text("say \"hi\"\nbye")], vec![Value::Null, text("")]];
    assert_eq!(render(&cols, &rows, &opts(TableFormat::Csv)), "a,b\n\"x,y\",\"say \"\"hi\"\"\nbye\"\n,\n");
}

#[test]
fn tsv_escapes_tabs_and_newlines_so_rows_stay_one_line() {
    let cols = vec![Column::new("a", "text")];
    let rows = vec![vec![text("x\ty\nz\\")]];
    assert_eq!(render(&cols, &rows, &opts(TableFormat::Tsv)), "a\nx\\ty\\nz\\\\\n");
}

#[test]
fn json_preserves_types() {
    let cols = vec![
        Column::new("i", "int8"),
        Column::new("n", "numeric"),
        Column::new("b", "bool"),
        Column::new("j", "jsonb"),
        Column::new("bad", "json"),
        Column::new("bytes", "bytea"),
        Column::new("t", "text"),
        Column::new("z", "text"),
        Column::new("f", "float8"),
    ];
    let rows = vec![vec![
        Value::Int(5),
        text("12345678901234567890.123"),
        text("t"),
        text(r#"{"k": [1, 2]}"#),
        text("{not json"),
        Value::Bytes(vec![0xde, 0xad]),
        text("42"),
        Value::Null,
        Value::Float(2.5),
    ]];
    let out = render(&cols, &rows, &opts(TableFormat::Json));
    assert!(out.contains("\"n\": 12345678901234567890.123"), "exact numeric text kept verbatim: {out}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    let o = &v[0];
    assert_eq!(o["i"], serde_json::json!(5));
    assert!(o["n"].is_number());
    assert_eq!(o["b"], serde_json::json!(true));
    assert_eq!(o["j"], serde_json::json!({"k": [1, 2]}));
    assert_eq!(o["bad"], serde_json::json!("{not json"));
    assert_eq!(o["bytes"], serde_json::json!("0xdead"));
    assert_eq!(o["t"], serde_json::json!("42"), "text that looks numeric stays a string");
    assert!(o["z"].is_null());
    assert_eq!(o["f"], serde_json::json!(2.5));
}

#[test]
fn jsonl_is_one_object_per_line() {
    let (c, r) = sample();
    let out = render(&c, &r, &opts(TableFormat::JsonLines));
    assert_eq!(out.lines().count(), 2);
    for line in out.lines() {
        assert!(serde_json::from_str::<serde_json::Value>(line).unwrap().is_object());
    }
    assert_eq!(render(&c, &[], &opts(TableFormat::Json)), "[]\n");
}

#[test]
fn a_machine_format_printed_in_parts_is_the_whole_result() {
    let (c, r) = sample();
    for (_, format) in TableFormat::ALL.iter().filter(|(_, f)| f.is_machine()) {
        let o = opts(*format);
        let (mut stream, head) = Stream::open(c.clone(), o.clone());
        let parts = head + &stream.rows(&r[..1]) + &stream.rows(&[]) + &stream.rows(&r[1..]) + &stream.close();
        assert_eq!(parts, render(&c, &r, &o), "{format:?}");
        let (stream, head) = Stream::open(c.clone(), o.clone());
        assert_eq!(head + &stream.close(), render(&c, &[], &o), "{format:?} with no rows");
    }
}

#[test]
fn html_escapes_markup() {
    let cols = vec![Column::new("<c>", "text")];
    let rows = vec![vec![text("<b>&\"'")], vec![Value::Null]];
    let out = render(&cols, &rows, &opts(TableFormat::Html));
    assert!(out.contains("<th>&lt;c&gt;</th>"));
    assert!(out.contains("<td>&lt;b&gt;&amp;&quot;&#39;</td>"));
    assert!(out.contains("<td class=\"null\">NULL</td>"));
    assert!(!out.contains("<b>"));
}

#[test]
fn sql_insert_escapes_per_backend() {
    let cols = vec![Column::new("id", "int"), Column::new("name", "text")];
    let rows = vec![vec![Value::Int(1), text(r"O'Re\illy")]];
    let pg = OutputOptions { table_name: Some("app.users".into()), ..opts(TableFormat::SqlInsert) };
    assert_eq!(render(&cols, &rows, &pg), "INSERT INTO app.users (id, name) VALUES (1, 'O''Re\\illy');\n");
    let my = OutputOptions { backend: Backend::MySql, ..opts(TableFormat::SqlInsert) };
    assert_eq!(render(&cols, &rows, &my), "INSERT INTO `table` (id, name) VALUES (1, 'O''Re\\\\illy');\n");
}

#[test]
fn sql_update_keys_on_first_column() {
    let cols = vec![Column::new("id", "int"), Column::new("v", "text")];
    let rows = vec![vec![Value::Int(1), text("a")], vec![Value::Null, Value::Null]];
    let o = OutputOptions { table_name: Some("t".into()), ..opts(TableFormat::SqlUpdate) };
    assert_eq!(
        render(&cols, &rows, &o),
        "UPDATE t SET v = 'a' WHERE id = 1;\nUPDATE t SET v = NULL WHERE id IS NULL;\n"
    );
}

/// Expanded output keeps the look of the table style in use: a wide result in `rounded` must not
/// fall back to mysql's `*** 1. row ***` banners.
#[test]
fn expanded_rounded_output_is_a_framed_record_per_row() {
    let (c, r) = sample();
    let on = OutputOptions { expanded: Expanded::On, terminal_width: 40, ..opts(TableFormat::Rounded) };
    let expected = "\
╭─ 1 ────┬───────
│     id │ 1
│   name │ alice
│   note │ NULL
│ amount │ 12.50
├─ 2 ────┼───────
│     id │ 22
│   name │ 日本語
│   note │ two
│        │ lines
│ amount │ 3
╰────────┴───────
";
    assert_eq!(render(&c, &r, &on), expected);
    // A table wider than the terminal switches too, and the rules shrink to fit.
    let auto = OutputOptions { expanded: Expanded::Auto, terminal_width: 12, ..opts(TableFormat::Rounded) };
    assert_eq!(render(&c, &r, &auto), expected.replace("───────\n", "────\n"));
}

#[test]
fn vertical_golden_right_aligned_names() {
    let (c, r) = sample();
    let expected = "\
*************************** 1. row ***************************
    id: 1
  name: alice
  note: NULL
amount: 12.50
*************************** 2. row ***************************
    id: 22
  name: 日本語
  note: two
        lines
amount: 3
";
    assert_eq!(render(&c, &r, &opts(TableFormat::Vertical)), expected);
    let on = OutputOptions { expanded: Expanded::On, ..opts(TableFormat::Psql) };
    assert_eq!(render(&c, &r, &on), expected);
}

#[test]
fn auto_expanded_switches_to_vertical_only_when_too_wide() {
    let (c, r) = sample();
    let narrow = OutputOptions { expanded: Expanded::Auto, terminal_width: 20, ..opts(TableFormat::Rounded) };
    let record = |s: String| s.starts_with("╭─ 1 ");
    assert!(record(render(&c, &r, &narrow)));
    let wide = OutputOptions { terminal_width: 200, ..narrow.clone() };
    assert!(!record(render(&c, &r, &wide)));
    let exact = OutputOptions { terminal_width: 32, ..narrow.clone() };
    assert!(!record(render(&c, &r, &exact)), "a table exactly as wide as the terminal fits");
    let psql = OutputOptions { format: TableFormat::Psql, ..narrow.clone() };
    assert!(render(&c, &r, &psql).starts_with("*****"), "unboxed formats keep mysql-style records");
    let machine = OutputOptions { format: TableFormat::Csv, ..narrow };
    assert!(render(&c, &r, &machine).starts_with("id,"));
}

#[test]
fn truncation_uses_ellipsis_and_respects_display_width() {
    let cols = vec![Column::new("s", "text")];
    let rows = vec![vec![text("abcdefghij")], vec![text("日本語日本語")]];
    let o = OutputOptions { max_field_width: Some(5), ..opts(TableFormat::Plain) };
    assert_eq!(render(&cols, &rows, &o), "s\nabcd…\n日本…\n");
    let csv = OutputOptions { format: TableFormat::Csv, ..o };
    assert!(render(&cols, &rows, &csv).contains("abcdefghij"), "machine formats are never truncated");
}

#[test]
fn control_chars_are_made_visible() {
    let cols = vec![Column::new("s", "text")];
    let rows = vec![vec![text("a\x1b[31mb\r\nc\x07")]];
    let out = render(&cols, &rows, &opts(TableFormat::Plain));
    assert!(!out.contains('\x1b') && !out.contains('\x07') && !out.contains('\r'));
    assert_eq!(out, "s\na␛[31mb\nc␇\n");
}

#[test]
fn colors_never_leak_into_machine_formats() {
    let (c, r) = sample();
    for (_, f) in TableFormat::ALL.iter().filter(|(_, f)| f.is_machine()) {
        let out = render(&c, &r, &colored(*f));
        assert!(!out.contains('\x1b'), "{f:?} contains escapes");
        assert_eq!(out, render(&c, &r, &opts(*f)));
    }
}

#[test]
fn colored_output_has_identical_layout_once_escapes_are_stripped() {
    let (c, mut r) = sample();
    r.push(vec![Value::Int(3), Value::Bool(true), text("x"), Value::Null]);
    for (_, f) in TableFormat::ALL.iter().filter(|(_, f)| !f.is_machine()) {
        let plain = render(&c, &r, &opts(*f));
        let col = render(&c, &r, &colored(*f));
        assert!(col.contains('\x1b'), "{f:?} should be colored");
        assert_eq!(strip_ansi(&col), plain, "{f:?}");
    }
}

#[test]
fn null_and_numbers_get_theme_colors() {
    let (c, r) = sample();
    let o = colored(TableFormat::Plain);
    let out = render(&c, &r, &o);
    let null = format!("{}{}NULL{}", theme::ITALIC, theme::ansi_fg(o.theme.null, o.color), theme::RESET);
    let num = format!("{}12.50{}", theme::ansi_fg(o.theme.number, o.color), theme::RESET);
    assert!(out.contains(&null) && out.contains(&num), "{out:?}");
    assert!(out.contains(" alice "), "plain text uses the terminal default color");
}

#[test]
fn untyped_numeric_values_are_right_aligned() {
    let cols = vec![Column::new("x", ""), Column::new("y", "")];
    let rows = vec![vec![Value::Int(5), text("a")], vec![Value::Int(100), text("bbb")]];
    assert_eq!(render(&cols, &rows, &opts(TableFormat::Plain)), "  x  y\n  5  a\n100  bbb\n");
    let left = OutputOptions { align_numbers: false, ..opts(TableFormat::Plain) };
    assert_eq!(render(&cols, &rows, &left), "x    y\n5    a\n100  bbb\n");
}

#[test]
fn empty_result_renders_header_only() {
    let (c, _) = sample();
    assert_eq!(render(&c, &[], &opts(TableFormat::Psql)), " id | name | note | amount\n----+------+------+--------\n");
    assert_eq!(render(&[], &[], &opts(TableFormat::Psql)), "");
}

#[test]
fn status_lines() {
    let o = opts(TableFormat::Rounded);
    let rows = Summary { status: Some("SELECT 42".into()), rows_affected: Some(42), ..Default::default() };
    assert_eq!(render_status(&rows, 42, Some(Duration::from_micros(12_340)), &o), "42 rows · 12.3 ms");
    assert_eq!(render_status(&Summary::default(), 1, None, &o), "1 row");
    let empty_select = Summary { status: Some("SELECT 0".into()), ..Default::default() };
    assert_eq!(render_status(&empty_select, 0, None, &o), "0 rows");
    let dml = Summary { rows_affected: Some(3), ..Default::default() };
    assert_eq!(render_status(&dml, 0, Some(Duration::from_micros(4100)), &o), "Query OK, 3 rows affected · 4.1 ms");
    let tag = Summary { status: Some("INSERT 0 1".into()), rows_affected: Some(1), ..Default::default() };
    assert_eq!(render_status(&tag, 0, None, &o), "INSERT 0 1");
    let warn = Summary { rows_affected: Some(1), warnings: 2, ..Default::default() };
    assert_eq!(render_status(&warn, 0, None, &o), "Query OK, 1 row affected, 2 warnings");
    assert_eq!(render_status_for(&Summary::default(), 0, true, None, &o), "0 rows");
    let c = colored(TableFormat::Rounded);
    assert_eq!(strip_ansi(&render_status(&dml, 0, None, &c)), "Query OK, 3 rows affected");
    assert!(render_status(&dml, 0, None, &c).starts_with("\x1b["));
}

#[test]
fn durations() {
    assert_eq!(format_duration(Duration::from_micros(420)), "0.42 ms");
    assert_eq!(format_duration(Duration::from_millis(1500)), "1.50 s");
    assert_eq!(format_duration(Duration::from_millis(62_500)), "1m 2.5s");
}

#[test]
fn titled_renders_title_table_and_footer() {
    let (c, r) = sample();
    let t = Titled {
        title: Some("List".into()),
        result: crate::db::ResultSet::new(c, r),
        footer: Some("Indexes:\n    \"pk\" PRIMARY KEY".into()),
        text: None,
    };
    let out = render_titled(&t, &opts(TableFormat::Psql));
    assert!(out.starts_with(&format!("{}List\n id |", " ".repeat(13))), "title centered over the table:\n{out}");
    assert!(out.ends_with("Indexes:\n    \"pk\" PRIMARY KEY\n"));
    let text = Titled { text: Some("CREATE TABLE t();".into()), ..Default::default() };
    assert_eq!(render_titled(&text, &opts(TableFormat::Rounded)), "CREATE TABLE t();\n");
}

#[test]
fn cell_text_substitutes_null() {
    assert_eq!(cell_text(&Value::Null, "∅"), "∅");
    assert_eq!(cell_text(&Value::Int(3), "∅"), "3");
}

#[test]
#[ignore = "timing; run with --release -- --ignored"]
fn render_100k_rows_fast() {
    let cols: Vec<Column> = (0..10).map(|i| Column::new(format!("c{i}"), if i % 2 == 0 { "int" } else { "text" })).collect();
    let rows: Vec<Row> = (0..100_000)
        .map(|r| (0..10).map(|i| if i % 2 == 0 { Value::Int(r * i) } else { text("some text value") }).collect())
        .collect();
    let start = std::time::Instant::now();
    let out = render(&cols, &rows, &colored(TableFormat::Rounded));
    let elapsed = start.elapsed();
    assert!(out.len() > 1_000_000);
    assert!(elapsed < Duration::from_millis(1000), "took {elapsed:?}");
    println!("100k x 10 colored rounded: {elapsed:?}");
}
