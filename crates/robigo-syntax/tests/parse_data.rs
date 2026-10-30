use robigo_syntax::parse_data::record_parse_data;

fn unhex(s: &str) -> String {
    String::from_utf8((0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()).unwrap()
}

#[test]
fn raw_records_match_gnu_r_4_5_0() {
    let oracle = include_str!("../../../tests/parse/data.tsv");
    let mut source = String::new();
    let mut expected = String::new();
    let check = |source: &str, expected: &str| {
        if source.is_empty() {
            return;
        }
        let records = record_parse_data(source).unwrap_or_else(|e| panic!("{source:?}: {e}"));
        let mut got = String::new();
        for r in records {
            use std::fmt::Write;
            let text: String = r.text.bytes().map(|b| format!("{b:02x}")).collect();
            writeln!(
                got,
                "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t",
                r.line1,
                r.col1,
                r.line2,
                r.col2,
                i32::from(r.terminal()),
                r.token,
                r.id,
                r.parent,
                r.token_name(),
                text
            )
            .unwrap();
        }
        assert_eq!(got, expected, "source {source:?}");
    };
    for line in oracle.lines() {
        if let Some(hex) = line.strip_prefix("S\t") {
            check(&source, &expected);
            source = unhex(hex.trim());
            expected.clear();
        } else {
            expected.push_str(line);
            expected.push('\n');
        }
    }
    check(&source, &expected);
}
