//! Read GNU R's installed lazy-load database for base (if present) and
//! check that functions come back as closures with their source.

use std::io::Read;

use robigo_runtime::Interp;
use robigo_runtime::value::Value;

const BASE: &str = "/usr/lib/R/library/base/R/base";

fn gunzip(path: &str) -> Vec<u8> {
    let f = std::fs::File::open(path).unwrap();
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(f).read_to_end(&mut out).unwrap();
    out
}

#[test]
fn read_installed_base_rdb() {
    if !std::path::Path::new(&format!("{}.rdx", BASE)).exists() {
        eprintln!("GNU R not installed; skipping");
        return;
    }
    let mut interp = Interp::with_output(Box::new(std::io::sink()), Box::new(std::io::sink()));
    let idx = gunzip(&format!("{}.rdx", BASE));
    let map = robigo_runtime::serialize::unserialize(&mut interp, &idx, Value::Null).ok().expect("unserialize .rdx");
    let names = map.names().unwrap();
    let names: Vec<String> =
        robigo_runtime::coerce::to_str(&names).into_iter().map(|s| s.unwrap().to_string()).collect();
    assert_eq!(names, vec!["variables", "references", "compressed"]);
    let Value::List(l) = &map else { panic!() };
    let vars = &l.data[0];
    let var_names: Vec<String> =
        robigo_runtime::coerce::to_str(&vars.names().unwrap()).into_iter().map(|s| s.unwrap().to_string()).collect();
    let compressed = robigo_runtime::coerce::as_integer(&l.data[2]);
    let Value::List(keys) = vars else { panic!() };
    let mut f = std::fs::File::open(format!("{}.rdb", BASE)).unwrap();
    let mut rdb = Vec::new();
    f.read_to_end(&mut rdb).unwrap();
    let mut ok = 0;
    for name in ["paste", "sapply", "tryCatch", "print.default", "data.frame"] {
        let k = var_names.iter().position(|n| n == name).expect(name);
        let key = robigo_runtime::coerce::to_int(&keys.data[k], &mut Default::default());
        let (off, len) = (key[0] as usize, key[1] as usize);
        let data = robigo_runtime::builtins::serialize::decompress(&rdb[off..off + len], compressed).ok().unwrap();
        // References to environments go through a hook; use a stub.
        let v = robigo_runtime::serialize::unserialize(&mut interp, &data, Value::Null);
        match v {
            Ok(Value::Closure(c)) => {
                let text = robigo_runtime::deparse::deparse(&c.body).join("\n");
                assert!(!text.is_empty());
                ok += 1;
            }
            Ok(other) => panic!("{}: got {}", name, other.type_name()),
            Err(_) => panic!("{}: unserialize failed", name),
        }
    }
    assert_eq!(ok, 5);
}
