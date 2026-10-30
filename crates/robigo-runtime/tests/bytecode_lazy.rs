#[cfg(test)]
mod bytecode_lazy_tests {
    use robigo_runtime::{
        Interp, Value,
        serialize::{serialize, serialize_binary, unserialize},
        sym::Sym,
    };
    use std::rc::Rc;

    fn fixture(name: &str) -> (Vec<u8>, Value) {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/r/serialize/bytecode-fixtures")
            .join(name);
        let bytes = std::fs::read(path).unwrap();
        let mut interp = Interp::new();
        let Ok(value) = unserialize(&mut interp, &bytes, Value::Null) else { panic!("oracle bytecode fixture") };
        (bytes, value)
    }

    #[test]
    fn serialization_keeps_constants_lazy_before_and_after_inspection() {
        let (bytes, value) = fixture("body-code.rds");
        let Value::Bytecode(code) = &value else { panic!("bytecode fixture") };
        assert!(code.consts.get().is_none());
        assert!(code.code.get().is_none());
        assert!(!code.cells.encoded.is_empty());
        assert_eq!(serialize(&value), bytes);
        assert!(code.consts.get().is_none());
        assert!(!code.constants().is_empty());
        assert!(matches!(code.code(), Value::Int(_)));
        assert!(code.consts.get().is_some());
        assert_eq!(serialize(&value), bytes);
    }

    #[test]
    fn native_binary_import_replays_oracle_xdr_without_forcing() {
        for name in ["body-code.rds", "real-constants.rds", "string-constants.rds"] {
            let (bytes, value) = fixture(name);
            let native = serialize_binary(&value, 3);
            let mut interp = Interp::new();
            let Ok(value) = unserialize(&mut interp, &native, Value::Null) else { panic!("native bytecode") };
            let Value::Bytecode(code) = &value else { panic!("bytecode fixture") };
            assert!(code.code.get().is_none());
            assert!(code.consts.get().is_none());
            assert_eq!(serialize(&value), bytes);
            code.constants();
            code.code();
            assert_eq!(serialize(&value), bytes);
        }
    }

    #[test]
    fn deferred_real_constants_preserve_ieee_bits() {
        let (bytes, value) = fixture("real-constants.rds");
        let Value::Bytecode(code) = &value else { panic!("bytecode fixture") };
        assert_eq!(serialize(&value), bytes);
        let constants = code.constants();
        assert_eq!(constants.len(), 8);
        let expected = [
            42.0f64.to_bits(),
            (-0.0f64).to_bits(),
            0x7ff00000000007a2,
            0x7ff8000000000000,
            f64::INFINITY.to_bits(),
            f64::NEG_INFINITY.to_bits(),
            2.5f64.to_bits(),
        ];
        for (value, bits) in constants.iter().zip(expected) {
            let Value::Real(v) = value else { panic!("oracle real") };
            assert_eq!(v.data[0].to_bits(), bits);
        }
        assert_eq!(serialize(&value), bytes);
    }

    #[test]
    fn deferred_strings_preserve_encoding_and_na() {
        let (bytes, value) = fixture("string-constants.rds");
        let Value::Bytecode(code) = &value else { panic!("bytecode fixture") };
        assert!(code.consts.get().is_none());
        assert_eq!(serialize(&value), bytes);
        let pool = code.constants();
        assert_eq!(pool.len(), 5);
        let Value::Str(v) = &pool[1] else { panic!("strings") };
        assert_eq!(v.data[0].as_ref().unwrap().as_ref(), "ascii");
        assert_eq!(v.data[1].as_ref().unwrap().as_ref(), "é");
        assert!(v.data[2].is_none());
        let Value::Str(latin) = &pool[2] else { panic!("latin1") };
        assert_eq!(latin.data[0].as_ref().unwrap().encoding_name(), "latin1");
        let Value::Str(raw) = &pool[3] else { panic!("bytes") };
        assert_eq!(&*raw.data[0].as_ref().unwrap().raw_bytes(), &[255, 128]);
        assert_eq!(serialize(&value), bytes);
    }

    #[test]
    fn attribute_copy_shares_immutable_bytecode_metadata() {
        let (bytes, value) = fixture("constant-code.rds");
        let mut alias = value.clone();
        alias.set_attr_raw(Sym::new("note"), Value::str("cow"));
        let (Value::Bytecode(before), Value::Bytecode(after)) = (&value, &alias) else { panic!("bytecode fixture") };
        assert!(!Rc::ptr_eq(before, after));
        assert!(Rc::ptr_eq(&before.cells, &after.cells));
        assert!(Rc::ptr_eq(&before.rep_values, &after.rep_values));
        assert!(before.consts.get().is_none());
        assert!(after.consts.get().is_none());
        assert_eq!(serialize(&value), bytes);
        before.constants();
        let mut forced_alias = value.clone();
        forced_alias.set_attr_raw(Sym::new("note"), Value::str("forced"));
        let Value::Bytecode(forced) = forced_alias else { panic!("bytecode fixture") };
        assert!(Rc::ptr_eq(before.consts.get().unwrap(), forced.consts.get().unwrap()));
    }
    #[test]
    fn deferred_constants_match_the_oracle_scalar_pool() {
        let (_, value) = fixture("scalar-constants.rds");
        let Value::Bytecode(code) = value else { panic!("bytecode fixture") };
        let mut named = Value::int(2);
        named.set_attr_raw(Sym::new("names"), Value::str("x"));
        let expected = Value::list(vec![
            Value::int(42),
            Value::lgl(true),
            Value::lgl(false),
            Value::lgl_na(),
            Value::int(i32::MIN),
            Value::int(-1),
            named,
            Value::ints(vec![1, 2]),
            Value::sym("x"),
            Value::Null,
        ]);
        let actual = Value::list(code.constants().to_vec());
        assert!(robigo_runtime::builtins::identical::identical(&actual, &expected));
    }
}
