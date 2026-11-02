//! Native routines of the grid package (src/library/grid/src).

use super::NativeFn;
use crate::error::RResult;
use crate::interp::Interp;
use crate::value::*;

pub static PORTS: &[(&str, NativeFn)] = &[
    ("initGrid", init_grid),
    ("killGrid", kill_grid),
    ("constructUnits", construct_units),
    ("asUnit", as_unit),
    ("conformingUnits", conforming_units),
    ("matchUnit", match_unit),
];

/// grid.c's `R_gridEvalEnv`: the package environment grid's C code
/// evaluates in. Thread-local because the runtime is single-interpreter.
thread_local! {
    static GRID_EVAL_ENV: std::cell::RefCell<Option<crate::env::Env>> = const { std::cell::RefCell::new(None) };
}

// R: grid/src/grid.c:L_initGrid — remember .GridEvalEnv and register the
// grid callback with the graphics engine. robigo has no graphics engine
// (no device can be opened), so there is nothing to register with; the
// eval environment is kept for the grid routines that evaluate in it.
fn init_grid(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &crate::env::Env) -> RResult<Value> {
    match &args[0].value {
        Value::Env(e) => GRID_EVAL_ENV.with(|g| *g.borrow_mut() = Some(e.clone())),
        _ => return Err(crate::error::Flow::error("robigo: initGrid without an evaluation environment")),
    }
    Ok(Value::Null)
}

// R: grid/src/grid.c:L_killGrid — unregister the callback; with no
// engine registration there is nothing to undo.
fn kill_grid(_: &mut Interp, _: &Value, _: Vec<Arg>, _: &crate::env::Env) -> RResult<Value> {
    Ok(Value::Null)
}

/// unit.c's `UnitTable`: name to LUnit code (codes over 1000 are
/// pseudonyms for `code - 1000`).
static UNIT_TABLE: &[(&str, i32)] = &[
    ("npc", 0),
    ("cm", 1),
    ("inches", 2),
    ("lines", 3),
    ("native", 4),
    ("null", 5),
    ("snpc", 6),
    ("mm", 7),
    ("points", 8),
    ("picas", 9),
    ("bigpts", 10),
    ("dida", 11),
    ("cicero", 12),
    ("scaledpts", 13),
    ("strwidth", 14),
    ("strheight", 15),
    ("strascent", 16),
    ("strdescent", 17),
    ("char", 18),
    ("grobx", 19),
    ("groby", 20),
    ("grobwidth", 21),
    ("grobheight", 22),
    ("grobascent", 23),
    ("grobdescent", 24),
    ("mylines", 103),
    ("mychar", 104),
    ("mystrwidth", 105),
    ("mystrheight", 106),
    ("sum", 201),
    ("min", 202),
    ("max", 203),
    ("centimetre", 1001),
    ("centimetres", 1001),
    ("centimeter", 1001),
    ("centimeters", 1001),
    ("in", 1002),
    ("inch", 1002),
    ("line", 1003),
    ("millimetre", 1007),
    ("millimetres", 1007),
    ("millimeter", 1007),
    ("millimeters", 1007),
    ("point", 1008),
    ("pt", 1008),
];

// R: grid/src/unit.c:convertUnit
fn convert_unit(name: &str) -> RResult<i32> {
    let mut code = UNIT_TABLE.iter().find(|&&(n, _)| n == name).map(|&(_, c)| c).unwrap_or(-1);
    if code < 0 {
        return Err(crate::error::Flow::error("Invalid unit"));
    }
    if code > 1000 {
        code -= 1000;
    }
    Ok(code)
}

fn is_string_unit(u: i32) -> bool {
    (14..=17).contains(&u)
}

fn is_grob_unit(u: i32) -> bool {
    (19..=24).contains(&u)
}

/// Evaluate a grid R function in `.GridEvalEnv` (validData's gPath and
/// depth calls).
fn eval_in_grid(i: &mut Interp, call: &Value) -> RResult<Value> {
    let env = GRID_EVAL_ENV.with(|g| g.borrow().clone());
    let Some(env) = env else {
        return Err(crate::error::Flow::error("robigo: grid is not initialized"));
    };
    i.eval(call, &env)
}

// R: grid/src/unit.c:validData — per-unit data checks; strings for
// strwidth/height, grobs or gPaths for grobwidth/height (strings
// upgraded through gPath(), gPaths checked with depth()), NULL for the
// rest.
fn valid_data(i: &mut Interp, data: &Value, units: &[i32], n: usize) -> RResult<Value> {
    let mut items = crate::coerce::to_list(data);
    let ndata = items.len();
    if ndata != 1 && ndata < n {
        return Err(crate::error::Flow::error(
            "data must be either NULL, have length 1, or match the length of the final unit vector",
        ));
    }
    for k in 0..units.len() {
        let single = items[k % ndata].clone();
        let u = units[k];
        let (string_unit, grob_unit) = (is_string_unit(u), is_grob_unit(u));
        if string_unit && !matches!(single, Value::Str(_) | Value::Expr(_)) {
            return Err(crate::error::Flow::error("no string supplied for 'strwidth/height' unit"));
        }
        if grob_unit {
            if !(single.inherits("grob") || single.inherits("gPath") || matches!(single, Value::Str(_))) {
                return Err(crate::error::Flow::error("no 'grob' supplied for 'grobwidth/height' unit"));
            }
            if let Value::Str(s) = &single {
                let text = s.data.first().and_then(|e| e.as_ref().map(|r| r.to_string())).unwrap_or_default();
                let call = Value::lang(Value::sym("gPath"), vec![Arg::pos(Value::str(&text))]);
                let gpath = eval_in_grid(i, &call)?;
                items[k % ndata] = gpath;
            }
            let single = items[k % ndata].clone();
            if single.inherits("gPath") {
                let call = Value::lang(Value::sym("depth"), vec![Arg::pos(single)]);
                let depth = eval_in_grid(i, &call)?;
                if crate::coerce::as_integer(&depth) > 1 {
                    return Err(crate::error::Flow::error("'gPath' must have depth 1 in 'grobwidth/height' units"));
                }
            }
        }
        if !string_unit && !grob_unit && !single.is_null() {
            return Err(crate::error::Flow::error("non-NULL value supplied for plain unit"));
        }
    }
    Ok(Value::list(items))
}

// R: grid/src/unit.c:constructUnits — grid's unit() constructor: a
// plain numeric with unit/class attributes when one non-special unit is
// given, else a list of (amount, data, code) triples with class
// c("unit", "unit_v2").
fn construct_units(i: &mut Interp, _: &Value, args: Vec<Arg>, _: &crate::env::Env) -> RResult<Value> {
    let amount = i.coerce(&args[0].value, crate::value::VType::Real)?;
    let data = &args[1].value;
    let units_arg = &args[2].value;
    let unit_names: Vec<String> = match units_arg {
        Value::Str(s) => s.data.iter().map(|e| e.as_ref().map(|r| r.to_string()).unwrap_or_default()).collect(),
        _ => return Err(crate::error::Flow::error("'units' must be character")),
    };
    if unit_names.is_empty() {
        return Err(crate::error::Flow::error("'units' must be of length > 0"));
    }
    let val_units: Vec<i32> = unit_names.iter().map(|n| convert_unit(n)).collect::<RResult<_>>()?;
    let amounts = crate::coerce::to_real(&amount, &mut Default::default());
    if val_units.len() == 1 && !is_string_unit(val_units[0]) && !is_grob_unit(val_units[0]) {
        let mut simple = amount;
        simple.set_attr_raw(crate::sym::Sym::new("unit"), Value::ints(val_units.clone()));
        simple.set_attr_raw(crate::sym::s::CLASS, Value::strs_from(["simpleUnit", "unit", "unit_v2"]));
        return Ok(simple);
    }
    let n = amounts.len().max(val_units.len());
    let validated = valid_data(i, data, &val_units, n)?;
    let items = crate::coerce::to_list(&validated);
    let ndata = items.len();
    let mut out = Vec::with_capacity(n);
    for k in 0..n {
        out.push(Value::list(vec![
            Value::real(amounts[k % amounts.len()]),
            items[k % ndata].clone(),
            Value::int(val_units[k % val_units.len()]),
        ]));
    }
    let mut units = Value::list(out);
    units.set_attr_raw(crate::sym::s::CLASS, Value::strs_from(["unit", "unit_v2"]));
    Ok(units)
}

// R: grid/src/unit.c:asUnit — upgrade a simpleUnit to the list form; a
// unit_v2 that is not simple is returned unchanged, anything without
// class "unit" is an error.
fn as_unit(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &crate::env::Env) -> RResult<Value> {
    let x = &args[0].value;
    if x.inherits("unit") {
        if !x.inherits("unit_v2") {
            return Err(crate::error::Flow::error("old version of unit class is no longer allowed"));
        }
        if !x.inherits("simpleUnit") {
            return Ok(x.clone());
        }
    } else {
        return Err(crate::error::Flow::error("object is not coercible to a unit"));
    }
    let amounts = crate::coerce::to_real(x, &mut Default::default());
    let val_unit = x.get_attr(crate::sym::Sym::new("unit")).unwrap_or(Value::Null);
    let out: Vec<Value> =
        amounts.iter().map(|&a| Value::list(vec![Value::real(a), Value::Null, val_unit.clone()])).collect();
    let mut units = Value::list(out);
    units.set_attr_raw(crate::sym::s::CLASS, Value::strs_from(["unit", "unit_v2"]));
    Ok(units)
}

// R: grid/src/unit.c:conformingUnits — the shared unit code when every
// element is a simpleUnit of one type, else NULL (with class checks).
fn conforming_units(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &crate::env::Env) -> RResult<Value> {
    let Value::List(items) = &args[0].value else {
        return Err(crate::error::Flow::error("object is not a unit"));
    };
    let mut unit_type: Option<i32> = None;
    for item in &items.data {
        if !item.inherits("unit") {
            return Err(crate::error::Flow::error("object is not a unit"));
        }
        if !item.inherits("unit_v2") {
            return Err(crate::error::Flow::error("old version of unit class is no longer allowed"));
        }
        if !item.inherits("simpleUnit") {
            return Ok(Value::Null);
        }
        let Some(Value::Int(codes)) = item.get_attr(crate::sym::Sym::new("unit")) else {
            return Ok(Value::Null);
        };
        let Some(&code) = codes.data.first() else {
            return Ok(Value::Null);
        };
        match unit_type {
            None => unit_type = Some(code),
            Some(t) if t != code => return Ok(Value::Null),
            _ => {}
        }
    }
    Ok(match unit_type {
        Some(t) => Value::int(t),
        None => Value::Null,
    })
}

// R: grid/src/unit.c:unitUnit — the code of unit `index` (simpleUnit's
// attribute, else the third cell of the unit_v2 3-list)
fn unit_unit(v: &Value, index: usize) -> Option<i32> {
    if v.inherits("simpleUnit") {
        if let Some(Value::Int(codes)) = v.get_attr(crate::sym::Sym::new("unit")) {
            return codes.data.first().copied();
        }
        return None;
    }
    let Value::List(items) = v else { return None };
    let scalar = items.data.get(index % items.data.len())?;
    let Value::List(cells) = scalar else { return None };
    match cells.data.get(2) {
        Some(Value::Int(code)) => code.data.first().copied(),
        _ => None,
    }
}

// R: grid/src/unit.c:matchUnit — the 1-based positions of units whose
// code equals `unit` (the arithmetic unit codes 201..203)
fn match_unit(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &crate::env::Env) -> RResult<Value> {
    let units = &args[0].value;
    let n = match units {
        Value::List(items) => items.data.len(),
        _ => return Err(crate::error::Flow::error("object is not a unit")),
    };
    let unit_int = crate::coerce::as_integer(&args[1].value);
    let mut matches = Vec::new();
    for k in 0..n {
        if unit_unit(units, k) == Some(unit_int) {
            matches.push((k + 1) as i32);
        }
    }
    Ok(Value::ints(matches))
}
