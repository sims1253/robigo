use robigo_numerics::{
    rand::{rsignrank, rwilcox},
    rng::{Rng, RngKind},
};

// Oracle: Debian trixie r-base-core 4.5.0-3. Re-record rank-rng.R with Rscript.
#[test]
fn rank_rng_oracle_grid() {
    for line in include_str!("data/rank-rng.txt").lines() {
        let fields: Vec<_> = line.split_whitespace().collect();
        let mut rng = Rng::default();
        rng.init(RngKind::MersenneTwister, fields[0].parse().unwrap()).unwrap();
        let parse = |s: &str| if s == "Inf" { f64::INFINITY } else { s.parse::<f64>().unwrap() };
        let a = parse(fields[1]);
        let values = fields.last().unwrap().split(',');
        for value in values {
            let actual =
                if fields.len() == 4 { rwilcox(&mut rng, a, parse(fields[2])) } else { rsignrank(&mut rng, a) };
            assert_eq!(actual.to_bits(), parse(value).to_bits(), "{line}");
        }
    }
}

#[test]
fn oversized_wilcox_reports_allocation_failure() {
    let mut rng = Rng::default();
    rng.init(RngKind::MersenneTwister, 1).unwrap();
    assert!(rwilcox(&mut rng, 2147483648.0, 1.0).is_nan());
    assert!(robigo_numerics::take_alloc_error().unwrap().contains("could not allocate memory"));
}
