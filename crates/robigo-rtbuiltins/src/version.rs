//! version.c's `.Internal(Version())`: the build's description, as GNU R
//! 4.6.2's configure and `Rversion.h` give it (`R_PLATFORM`, `R_CPU`,
//! `R_OS`, `R_MAJOR` ... `R_NICK`).

use robigo_rtcore::RSym;
use robigo_rtcore::{Args, Char, Cx, Encoding, Out, RResult, Ref, Root};

/// `Rversion.h`'s numbers for GNU R 4.6.2.
const MAJOR: &str = "4";
const MINOR: &str = "6.2";
const STATUS: &str = "";
const YEAR: &str = "2026";
const MONTH: &str = "10";
const DAY: &str = "29";
const SVN_REVISION: i32 = 90784;
const NICK: &str = "Diversified Portfolio";

/// configure's `R_CPU` and `R_OS`, and `R_PLATFORM` (the host triplet,
/// config.guess's), for the platforms robigo-rt builds on; None elsewhere.
fn platform() -> Option<(&'static str, &'static str, String)> {
    if !cfg!(all(target_os = "linux", target_env = "gnu")) {
        return None;
    }
    let cpu = std::env::consts::ARCH;
    // config.guess: x86_64-pc-linux-gnu, otherwise <cpu>-unknown-linux-gnu
    let vendor = if cpu == "x86_64" { "pc" } else { "unknown" };
    Some((cpu, "linux-gnu", format!("{cpu}-{vendor}-linux-gnu")))
}

/// `PrintVersionString`: "R version 4.6.2 (2026-10-29)", its form for a
/// release (an empty `R_STATUS`, a positive `R_SVN_REVISION`).
// R: version.c:PrintVersionString
fn version_string() -> String {
    format!("R version {MAJOR}.{MINOR} ({YEAR}-{MONTH}-{DAY})")
}

/// `PrintVersion_part_1`: the version string with its nickname, the
/// copyright line and the platform, each line ended ("R version 4.6.2
/// (2026-10-29) -- \"Diversified Portfolio\"\nCopyright (C) 2026 The R
/// Foundation for Statistical Computing\nPlatform: x86_64-pc-linux-gnu\n");
/// None on a platform robigo-rt does not describe (as `R.version`).
// R: version.c:PrintVersion_part_1
pub fn print_version_part_1() -> Option<String> {
    let (_, _, plat) = platform()?;
    let mut s = version_string();
    if !NICK.is_empty() {
        s.push_str(&format!(" -- \"{NICK}\""));
    }
    s.push_str(&format!("\nCopyright (C) {YEAR} The R Foundation for Statistical Computing\n"));
    s.push_str(&format!("Platform: {plat}"));
    // (no R_ARCH; a 64-bit build has no " (32-bit)")
    if size_of::<*const u8>() != 8 {
        s.push_str(&format!(" ({}-bit)", 8 * size_of::<*const u8>()));
    }
    s.push('\n');
    Some(s)
}

/// `.Internal(Version())`: a list of the build's strings (unix: 14).
// R: version.c:do_version
pub fn do_version(cx: &mut Cx, call: Root<'_>, args: Args<'_>, _env: Root<'_>, out: Out<'_>) -> RResult<()> {
    crate::row!("Version", true).check_arity(cx, call, args)?;
    let Some((cpu, os, plat)) = platform() else {
        return Err(crate::util::not_yet(cx, Some(call), "R.version on this platform"));
    };
    let system = format!("{cpu}, {os}");
    let svn = SVN_REVISION.to_string();
    let vs = version_string();
    let fields: [(&str, &str); 14] = [
        ("platform", &plat),
        ("arch", cpu),
        ("os", os),
        ("system", &system),
        ("status", STATUS),
        ("major", MAJOR),
        ("minor", MINOR),
        ("year", YEAR),
        ("month", MONTH),
        ("day", DAY),
        ("svn rev", &svn),
        ("language", "R"),
        ("version.string", &vs),
        ("nickname", NICK),
    ];
    cx.build(out, |cx| {
        let mut value = cx.alloc_vector::<Ref<'_>>(fields.len())?;
        let mut names = cx.alloc_vector::<Char<'_>>(fields.len())?;
        for (i, (n, v)) in fields.iter().enumerate() {
            names.set(i, cx.mk_char(n.as_bytes(), Encoding::Native)?);
            value.set(i, cx.mk_string(v.as_bytes(), Encoding::Native)?);
        }
        let mut a = crate::attrib::Attrs::new();
        a.install(crate::attrib::Attr { tag: cx.sym(RSym::Names), value: names.finish().into() });
        a.apply(cx, &mut value)?;
        Ok(value.finish().into())
    })
}
