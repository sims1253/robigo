//! String collation: GNU R's `Scollate` and the ICU collator behind it, with
//! the state that `icuSetCollate`, `icuGetCollate` and `Sys.setlocale`
//! change (R: util.c, platform.c:do_setlocale).
//!
//! GNU R built with ICU compares strings with an ICU collator for the
//! collation locale, opened on the first comparison, unless that locale is
//! "C"; with the collator disabled it uses `strcoll`, and after
//! `icuSetCollate(locale = "ASCII")` `strcmp`. robigo loads the system's ICU
//! at run time, the library GNU R links, so orders are GNU R's exactly,
//! tailorings and all. Without one robigo is GNU R built without ICU: every
//! comparison is `strcoll`, and `icuSetCollate` only warns.
//!
//! The interface is [`Scollate`], one per operation, whose comparisons do
//! the first-use setup as GNU R does and keep its error for
//! [`Scollate::finish`]. In the root collation (every locale ICU has no
//! tailoring for, such as C.UTF-8 and English) ASCII strings are compared
//! with ICU's weights from a table, checked against the collator before it
//! is used, so the common case costs no call into ICU.

use std::cell::{OnceCell, RefCell};
use std::cmp::Ordering;
use std::rc::Rc;

use crate::error::{Flow, RResult};

/// R: util.c:collationLocaleSet.
#[derive(Clone, Copy, PartialEq, Eq)]
enum LocaleSet {
    /// The collator is opened on the next comparison.
    Pending,
    /// Settled: the collator if there is one, else `strcoll`.
    Settled,
    /// `icuSetCollate(locale = "ASCII")`: `strcmp`.
    Ascii,
}

struct State {
    set: LocaleSet,
    /// R: util.c:collator.
    collator: Option<Rc<icu::Collator>>,
    /// Whether the ASCII table agrees with `collator`, once checked.
    root_ascii: Option<bool>,
    /// The comparison the state amounts to, until the state or the
    /// collation locale changes.
    current: Option<Rc<Collation>>,
}

thread_local! {
    static STATE: RefCell<State> = const {
        RefCell::new(State { set: LocaleSet::Pending, collator: None, root_ascii: None, current: None })
    };
}

/// Whether ICU is available: `capabilities("ICU")`.
pub fn icu_available() -> bool {
    icu::lib().is_some()
}

/// ICU's version, as `extSoftVersion()` reports it; "" without ICU.
pub fn icu_version() -> String {
    icu::version().unwrap_or_default()
}

/// A string comparison in the session's collation order.
pub struct Collation(Kind);

enum Kind {
    /// `strcmp`, and `strcoll` in the C locale.
    Bytes,
    Strcoll,
    Icu(Rc<icu::Collator>),
    /// ICU's root collation, ASCII from [`ROOT_ASCII`].
    IcuRoot(Rc<icu::Collator>),
}

impl Collation {
    /// R: util.c:Scollate once the collator is settled. Strings can collate
    /// equal without being identical.
    #[inline(always)]
    pub fn cmp(&self, a: &str, b: &str) -> Ordering {
        match &self.0 {
            Kind::Bytes => a.cmp(b),
            Kind::Strcoll => crate::sys::strcoll(a, b),
            Kind::IcuRoot(coll) => match root_ascii_cmp(a, b) {
                Some(o) => o,
                None => coll.cmp(a, b),
            },
            Kind::Icu(coll) => coll.cmp(a, b),
        }
    }
}

/// R: util.c:Scollate for the comparisons of one operation. The collation
/// is looked up at the first comparison of two different strings, which is
/// when GNU R opens its collator; an error doing so is kept for
/// [`Scollate::finish`], and the comparisons until then are not meaningful.
#[derive(Default)]
pub struct Scollate {
    coll: OnceCell<Option<Rc<Collation>>>,
    err: RefCell<Option<Flow>>,
}

impl Scollate {
    pub fn new() -> Self {
        Self::default()
    }

    #[inline(always)]
    pub fn cmp(&self, a: &str, b: &str) -> Ordering {
        if a == b {
            return Ordering::Equal;
        }
        match self.coll.get_or_init(|| self.open()) {
            Some(c) => c.cmp(a, b),
            None => Ordering::Equal,
        }
    }

    /// [`Scollate::cmp`], with the error of the first comparison.
    #[inline]
    pub fn try_cmp(&self, a: &str, b: &str) -> RResult<Ordering> {
        let o = self.cmp(a, b);
        match self.err.borrow_mut().take() {
            Some(e) => Err(e),
            None => Ok(o),
        }
    }

    #[cold]
    fn open(&self) -> Option<Rc<Collation>> {
        match current() {
            Ok(c) => Some(c),
            Err(e) => {
                *self.err.borrow_mut() = Some(e);
                None
            }
        }
    }

    /// The error the first comparison raised, if any: the operation's
    /// result must then be dropped.
    pub fn finish(self) -> RResult<()> {
        match self.err.into_inner() {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }
}

/// Sorts `v` stably by `key` in the collation order (GNU R's `ssort`,
/// `sortVector` and `orderVector1` on strings, through `Scollate`).
pub fn sort_by_key<T>(v: &mut [T], key: impl Fn(&T) -> &str) -> RResult<()> {
    let coll = Scollate::new();
    v.sort_by(|a, b| coll.cmp(key(a), key(b)));
    coll.finish()
}

/// The session's collation, setting it up first if no comparison has yet
/// (R: util.c:Scollate up to the comparison).
pub fn current() -> RResult<Rc<Collation>> {
    STATE.with(|st| {
        if let Some(c) = &st.borrow().current {
            return Ok(c.clone());
        }
        let mut st = st.borrow_mut();
        let st = &mut *st;
        if st.set == LocaleSet::Pending && icu::lib().is_some() {
            st.set = LocaleSet::Settled;
            st.collator = open_on_first_use()?.map(Rc::new);
            st.root_ascii = None;
        }
        let kind = match &st.collator {
            Some(coll) => {
                if *st.root_ascii.get_or_insert_with(|| root_ascii_agrees(coll)) {
                    Kind::IcuRoot(coll.clone())
                } else {
                    Kind::Icu(coll.clone())
                }
            }
            None if st.set == LocaleSet::Ascii => Kind::Bytes,
            None => match crate::sys::locale(libc::LC_COLLATE).as_deref() {
                Some("C" | "POSIX") | None => Kind::Bytes,
                Some(_) => Kind::Strcoll,
            },
        };
        let c = Rc::new(Collation(kind));
        st.current = Some(c.clone());
        Ok(c)
    })
}

/// R: util.c:Scollate's setup: ICU's collator for the collation locale,
/// unless the environment or the locale asks for C's order.
fn open_on_first_use() -> RResult<Option<icu::Collator>> {
    crate::builtins::sysutils::init_locale_on_evaluator();
    // R: "when LC_ALL is set to "C" (or unset and LC_COLLATE is set to "C"),
    // the "C" collation order will be used"
    let env = |v| std::env::var_os(v).filter(|s| !s.is_empty());
    let use_c = env("LC_ALL").or_else(|| env("LC_COLLATE")).is_some_and(|l| l == "C");
    let locale = icu_locale();
    if use_c || locale == "C" {
        return Ok(None);
    }
    match icu::Collator::open_default(&locale) {
        Ok(c) => Ok(Some(c)),
        Err(icu::OpenError::Locale(e)) => Err(Flow::error(format!("failed to set ICU locale ({})", e))),
        Err(icu::OpenError::Collator(e)) => Err(Flow::error(format!("failed to open ICU collator ({})", e))),
    }
}

/// R: util.c:getLocale: `R_ICU_LOCALE`, else the collation locale.
fn icu_locale() -> String {
    match std::env::var("R_ICU_LOCALE") {
        Ok(l) if !l.is_empty() => l,
        _ => crate::sys::locale(libc::LC_COLLATE).unwrap_or_else(|| "C".to_string()),
    }
}

/// `Sys.setlocale` changed the collation locale (R: util.c:resetICUcollator,
/// called by platform.c:do_setlocale): the collator is closed and opened
/// again at the next comparison, or, when `disable` (the locale is "C"),
/// comparisons use `strcoll`.
pub fn reset(disable: bool) {
    if icu::lib().is_none() {
        return;
    }
    STATE.with(|st| {
        let mut st = st.borrow_mut();
        st.collator = None;
        st.set = if disable { LocaleSet::Settled } else { LocaleSet::Pending };
        st.current = None;
    });
}

/// The C library's locale changed: what `strcoll` does may have.
pub fn locale_changed() {
    STATE.with(|st| st.borrow_mut().current = None);
}

/// `icuSetCollate(locale = s)` (R: util.c:do_ICUset). Needs ICU.
pub fn set_locale(s: &str) -> RResult<()> {
    STATE.with(|st| {
        let mut st = st.borrow_mut();
        st.collator = None;
        st.root_ascii = None;
        st.current = None;
        if s == "ASCII" {
            st.set = LocaleSet::Ascii;
            return Ok(());
        }
        if s != "none" {
            let locale = if s == "default" { icu_locale() } else { s.to_string() };
            match icu::Collator::open_default(&locale) {
                Ok(c) => st.collator = Some(Rc::new(c)),
                Err(icu::OpenError::Locale(e)) => {
                    return Err(Flow::error(format!("failed to set ICU locale {} ({})", s, e)));
                }
                Err(icu::OpenError::Collator(e)) => {
                    return Err(Flow::error(format!("failed to open ICU collator ({})", e)));
                }
            }
        }
        st.set = LocaleSet::Settled;
        Ok(())
    })
}

/// R: util.c:ATtable. The entries with a trailing space never match, so
/// GNU R 4.5.0 ignores `strength = "primary"` and the like.
const AT_TABLE: &[(&str, i32)] = &[
    ("case_first", icu::CASE_FIRST),
    ("upper", icu::UPPER_FIRST),
    ("lower", icu::LOWER_FIRST),
    ("default ", -1),
    ("strength", 999),
    ("primary ", 0),
    ("secondary ", 1),
    ("tertiary ", 2),
    ("quaternary ", 3),
    ("identical ", 15),
    ("french_collation", icu::FRENCH_COLLATION),
    ("on", icu::ON),
    ("off", icu::OFF),
    ("normalization", icu::NORMALIZATION_MODE),
    ("alternate_handling", icu::ALTERNATE_HANDLING),
    ("non_ignorable", icu::NON_IGNORABLE),
    ("shifted", icu::SHIFTED),
    ("case_level", icu::CASE_LEVEL),
    ("hiragana_quaternary", icu::HIRAGANA_QUATERNARY_MODE),
];

/// `icuSetCollate(<name> = value)` for a name other than `locale` (R:
/// util.c:do_ICUset): sets the collator's attribute, if there is a
/// collator and both names are in R's table.
pub fn set_attribute(name: &str, value: &str) -> RResult<()> {
    let find = |s: &str| AT_TABLE.iter().find(|(n, _)| *n == s).map_or(-1, |&(_, v)| v);
    let (at, val) = (find(name), find(value));
    STATE.with(|st| {
        let mut st = st.borrow_mut();
        let Some(coll) = st.collator.clone() else { return Ok(()) };
        if at == 999 && val >= 0 {
            coll.set_strength(val);
        } else if at >= 0 && val >= 0 {
            coll.set_attribute(at, val).map_err(|_| Flow::error("failed to set ICU collator attribute"))?;
        } else {
            return Ok(());
        }
        st.root_ascii = None;
        st.current = None;
        Ok(())
    })
}

/// `.Internal(icuGetCollate(type))` (R: util.c:do_ICUget): the collator's
/// actual (`type()` 1) or valid (2) locale, "ASCII", or "ICU not in use".
pub fn get(r#type: impl FnOnce() -> i32) -> RResult<String> {
    STATE.with(|st| {
        let st = st.borrow();
        if st.set == LocaleSet::Ascii {
            return Ok("ASCII".to_string());
        }
        let Some(coll) = &st.collator else { return Ok("ICU not in use".to_string()) };
        let t = r#type();
        if !(1..=2).contains(&t) {
            return Err(Flow::error(format!("invalid '{}' value", "type")));
        }
        Ok(coll.locale(t == 2).unwrap_or_else(|| "unknown".to_string()))
    })
}

// ----------------------------------------------------------------------
// The root collation of ASCII

/// ICU's root collation order of the ASCII characters that have weights
/// (non-ignorable): whitespace, punctuation and symbols, digits, letters.
/// VT and FF have weights too but are left to ICU, as are the ignorable
/// control characters.
const ROOT_PUNCT: &[u8] = b"\t\n\r _-,;:!?.'\"()[]{}@*/\\&#%`^+<=>|~$";

/// Each byte's primary weight (0 for those left to ICU, and for the bytes
/// of characters beyond ASCII) and tertiary weight (1 for upper case).
static ROOT_ASCII: [(u16, u8); 256] = {
    let mut t = [(0, 0); 256];
    let mut i = 0;
    while i < ROOT_PUNCT.len() {
        t[ROOT_PUNCT[i] as usize] = (1 + i as u16, 0);
        i += 1;
    }
    let mut c = 0;
    while c < 10 {
        t[b'0' as usize + c] = (100 + c as u16, 0);
        c += 1;
    }
    let mut c = 0;
    while c < 26 {
        t[b'a' as usize + c] = (200 + c as u16, 0);
        t[b'A' as usize + c] = (200 + c as u16, 1);
        c += 1;
    }
    t
};

/// The root collator's order of two strings that differ only in ASCII
/// characters of the table and have ASCII neighbours where they differ, or
/// `None` when it takes ICU.
///
/// Every character in the table has one primary weight and no secondary
/// one, so the strings' weights line up character by character: the first
/// primary difference decides; failing that the longer string is greater
/// (it has more weights, unless its extra characters are ignorable), and
/// then the first tertiary difference, case. Characters both strings share
/// add the same weights to both, whatever they are, unless they contract
/// with a character that differs: the root collation's contractions with
/// ASCII characters ("l·", U+FDD1 before "4" or "L") all have a character
/// beyond ASCII next to the ASCII one, so a non-ASCII neighbour of a
/// difference also takes ICU.
#[inline(always)]
fn root_ascii_cmp(a: &str, b: &str) -> Option<Ordering> {
    let (x, y) = (a.as_bytes(), b.as_bytes());
    let n = x.len().min(y.len());
    let ascii_at = |s: &[u8], i: usize| s.get(i).is_none_or(|&c| c < 0x80);
    let mut tert = Ordering::Equal;
    for i in 0..n {
        let (p, q) = (x[i], y[i]);
        if p == q {
            continue;
        }
        // 0 also for bytes beyond ASCII
        let ((pp, pt), (qp, qt)) = (ROOT_ASCII[p as usize], ROOT_ASCII[q as usize]);
        if pp == 0 || qp == 0 || (i > 0 && x[i - 1] >= 0x80) || !ascii_at(x, i + 1) || !ascii_at(y, i + 1) {
            return None;
        }
        if pp != qp {
            return Some(pp.cmp(&qp));
        }
        if tert == Ordering::Equal {
            tert = pt.cmp(&qt);
        }
    }
    if x[n..].iter().chain(&y[n..]).any(|&c| ROOT_ASCII[c as usize].0 == 0)
        || (n > 0 && x[n - 1] >= 0x80 && x.len() != y.len())
    {
        return None;
    }
    Some(x.len().cmp(&y.len()).then(tert))
}

/// Whether [`root_ascii_cmp`] is `coll`'s order: `coll` is ICU's root
/// collation with its default attributes and orders the table's
/// characters as the table does.
fn root_ascii_agrees(coll: &icu::Collator) -> bool {
    if !coll.is_root_default() {
        return false;
    }
    let mut chars: Vec<u8> = (0..128u8).filter(|&c| ROOT_ASCII[c as usize].0 != 0).collect();
    chars.sort_by_key(|&c| ROOT_ASCII[c as usize]);
    let s = |c: &[u8]| String::from_utf8(c.to_vec()).unwrap_or_default();
    // The table's characters in strictly increasing order, so no two are
    // equal ...
    let increasing = chars.windows(2).all(|w| coll.cmp(&s(&w[..1]), &s(&w[1..])) == Ordering::Less);
    // ... and the cases of a letter differ only in the tertiary weight.
    increasing
        && (b'a'..=b'z').all(|c| coll.cmp(&s(&[c, b'b']), &s(&[c.to_ascii_uppercase(), b'a'])) == Ordering::Greater)
}

// ----------------------------------------------------------------------
// ICU through dlopen

#[allow(unsafe_code)] // ICU through dlopen
mod icu {
    use std::ffi::{CStr, CString, c_char, c_int, c_void};
    use std::sync::{Mutex, OnceLock};

    type UErrorCode = c_int;

    /// ICU's opaque `UCollator`.
    #[repr(C)]
    pub struct UCollator {
        _private: [u8; 0],
    }

    // R: util.c's copy of ucol.h (UColAttribute, UColAttributeValue)
    pub const FRENCH_COLLATION: i32 = 0;
    pub const ALTERNATE_HANDLING: i32 = 1;
    pub const CASE_FIRST: i32 = 2;
    pub const CASE_LEVEL: i32 = 3;
    pub const NORMALIZATION_MODE: i32 = 4;
    const STRENGTH: i32 = 5;
    pub const HIRAGANA_QUATERNARY_MODE: i32 = 6;
    const NUMERIC_COLLATION: i32 = 7;
    const TERTIARY: i32 = 2;
    pub const OFF: i32 = 16;
    pub const ON: i32 = 17;
    pub const SHIFTED: i32 = 20;
    pub const NON_IGNORABLE: i32 = 21;
    pub const LOWER_FIRST: i32 = 24;
    pub const UPPER_FIRST: i32 = 25;
    const ACTUAL_LOCALE: c_int = 0;
    const VALID_LOCALE: c_int = 1;
    const REORDER_CODE_PUNCTUATION: c_int = 0x1001;

    pub struct Lib {
        uloc_set_default: unsafe extern "C" fn(*const c_char, *mut UErrorCode),
        uloc_get_default: unsafe extern "C" fn() -> *const c_char,
        open: unsafe extern "C" fn(*const c_char, *mut UErrorCode) -> *mut UCollator,
        close: unsafe extern "C" fn(*mut UCollator),
        set_strength: unsafe extern "C" fn(*mut UCollator, c_int),
        set_attribute: unsafe extern "C" fn(*mut UCollator, c_int, c_int, *mut UErrorCode),
        get_attribute: unsafe extern "C" fn(*const UCollator, c_int, *mut UErrorCode) -> c_int,
        get_max_variable: unsafe extern "C" fn(*const UCollator) -> c_int,
        get_reorder_codes: unsafe extern "C" fn(*const UCollator, *mut i32, i32, *mut UErrorCode) -> i32,
        get_locale_by_type: unsafe extern "C" fn(*const UCollator, c_int, *mut UErrorCode) -> *const c_char,
        strcoll_utf8:
            unsafe extern "C" fn(*const UCollator, *const c_char, i32, *const c_char, i32, *mut UErrorCode) -> c_int,
        get_version: unsafe extern "C" fn(*mut u8),
        version_to_string: unsafe extern "C" fn(*const u8, *mut c_char),
    }

    /// The libraries to try: the development link, then the versions ICU
    /// has had (50, which added `ucol_strcollUTF8`, on) newest first.
    fn candidates() -> Vec<(CString, Option<u32>)> {
        let mut v = Vec::new();
        if cfg!(target_os = "macos") {
            v.push((c"libicucore.A.dylib".to_owned(), None));
        } else {
            v.push((c"libicui18n.so".to_owned(), None));
            for n in (50..=99).rev() {
                v.push((CString::new(format!("libicui18n.so.{n}")).unwrap_or_default(), Some(n)));
            }
        }
        v
    }

    pub fn lib() -> Option<&'static Lib> {
        static LIB: OnceLock<Option<Lib>> = OnceLock::new();
        // SAFETY: ICU's C API, each symbol transmuted to the type its `Lib`
        // field declares from ucol.h, uloc.h and uversion.h. ICU renames
        // its symbols with the major version (`ucol_open_78`) unless built
        // not to.
        LIB.get_or_init(|| unsafe {
            for (name, major) in candidates() {
                let h = libc::dlopen(name.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL);
                if h.is_null() {
                    continue;
                }
                let suffixes: Vec<String> = match major {
                    Some(n) => vec![format!("_{n}"), String::new()],
                    None => (50..=99).rev().map(|n| format!("_{n}")).chain([String::new()]).collect(),
                };
                let Some(suffix) = suffixes.into_iter().find(|s| {
                    let n = CString::new(format!("ucol_strcollUTF8{s}")).unwrap_or_default();
                    !libc::dlsym(h, n.as_ptr()).is_null()
                }) else {
                    libc::dlclose(h);
                    continue;
                };
                let sym = |n: &str| {
                    let n = CString::new(format!("{n}{suffix}")).unwrap_or_default();
                    libc::dlsym(h, n.as_ptr())
                };
                macro_rules! sym {
                    ($n:literal) => {{
                        let p: *mut c_void = sym($n);
                        if p.is_null() {
                            libc::dlclose(h);
                            continue;
                        }
                        #[allow(clippy::missing_transmute_annotations)] // the type of the field
                        std::mem::transmute::<*mut c_void, _>(p)
                    }};
                }
                return Some(Lib {
                    uloc_set_default: sym!("uloc_setDefault"),
                    uloc_get_default: sym!("uloc_getDefault"),
                    open: sym!("ucol_open"),
                    close: sym!("ucol_close"),
                    set_strength: sym!("ucol_setStrength"),
                    set_attribute: sym!("ucol_setAttribute"),
                    get_attribute: sym!("ucol_getAttribute"),
                    get_max_variable: sym!("ucol_getMaxVariable"),
                    get_reorder_codes: sym!("ucol_getReorderCodes"),
                    get_locale_by_type: sym!("ucol_getLocaleByType"),
                    strcoll_utf8: sym!("ucol_strcollUTF8"),
                    get_version: sym!("u_getVersion"),
                    version_to_string: sym!("u_versionToString"),
                });
            }
            None
        })
        .as_ref()
    }

    /// R: platform.c:do_eSoftVersion's ICU entry.
    pub fn version() -> Option<String> {
        let lib = lib()?;
        let mut v = [0u8; 4]; // U_MAX_VERSION_LENGTH
        let mut s = [0 as c_char; 20]; // U_MAX_VERSION_STRING_LENGTH
        // SAFETY: the buffers have the sizes uversion.h gives, and
        // u_versionToString terminates what it writes.
        unsafe {
            (lib.get_version)(v.as_mut_ptr());
            (lib.version_to_string)(v.as_ptr(), s.as_mut_ptr());
            Some(CStr::from_ptr(s.as_ptr()).to_string_lossy().into_owned())
        }
    }

    pub enum OpenError {
        /// `uloc_setDefault` failed with this code.
        Locale(i32),
        /// `ucol_open` failed with this code.
        Collator(i32),
    }

    /// An open `UCollator`, closed on drop.
    pub struct Collator {
        lib: &'static Lib,
        ptr: *mut UCollator,
        /// ICU's default locale when the collator was opened had keywords
        /// (`@collation=...`).
        keywords: bool,
    }

    impl Drop for Collator {
        fn drop(&mut self) {
            // SAFETY: an open collator that nothing else refers to.
            unsafe { (self.lib.close)(self.ptr) }
        }
    }

    /// ICU's default locale is process-wide: held from setting it to
    /// opening the collator for it.
    static DEFAULT_LOCALE: Mutex<()> = Mutex::new(());

    impl Collator {
        /// `uloc_setDefault(locale)` and `ucol_open(NULL)`, as GNU R opens
        /// its collator. Needs ICU.
        pub fn open_default(locale: &str) -> Result<Collator, OpenError> {
            let Some(lib) = lib() else { return Err(OpenError::Collator(-1)) };
            let locale = locale.as_bytes();
            let n = locale.iter().position(|&c| c == 0).unwrap_or(locale.len());
            let locale = CString::new(&locale[..n]).unwrap_or_default();
            let _guard = DEFAULT_LOCALE.lock().unwrap_or_else(|e| e.into_inner());
            let mut status: UErrorCode = 0;
            // SAFETY: a NUL-terminated locale ID and a status to write; a
            // null locale opens the default locale's collator. The default
            // locale's name is copied before the lock is released.
            unsafe {
                (lib.uloc_set_default)(locale.as_ptr(), &mut status);
                if status > 0 {
                    return Err(OpenError::Locale(status));
                }
                let ptr = (lib.open)(std::ptr::null(), &mut status);
                if status > 0 || ptr.is_null() {
                    if !ptr.is_null() {
                        (lib.close)(ptr);
                    }
                    return Err(OpenError::Collator(status));
                }
                let default = (lib.uloc_get_default)();
                let keywords = default.is_null() || CStr::from_ptr(default).to_bytes().contains(&b'@');
                Ok(Collator { lib, ptr, keywords })
            }
        }

        /// `ucol_setStrength`, which reports no error.
        pub fn set_strength(&self, strength: i32) {
            // SAFETY: an open collator; ICU ignores strengths it does not have.
            unsafe { (self.lib.set_strength)(self.ptr, strength) }
        }

        /// `ucol_setAttribute`: the error code when it fails.
        pub fn set_attribute(&self, attr: i32, value: i32) -> Result<(), i32> {
            let mut status: UErrorCode = 0;
            // SAFETY: an open collator; ICU checks the attribute and value.
            unsafe { (self.lib.set_attribute)(self.ptr, attr, value, &mut status) };
            if status > 0 { Err(status) } else { Ok(()) }
        }

        /// `ucol_getLocaleByType`: the actual locale, or with `valid` the
        /// valid one; `None` when ICU has none.
        pub fn locale(&self, valid: bool) -> Option<String> {
            let mut status: UErrorCode = 0;
            let t = if valid { VALID_LOCALE } else { ACTUAL_LOCALE };
            // SAFETY: an open collator. The result is null or a C string
            // that lives as long as the collator.
            unsafe {
                let p = (self.lib.get_locale_by_type)(self.ptr, t, &mut status);
                (status <= 0 && !p.is_null()).then(|| CStr::from_ptr(p).to_string_lossy().into_owned())
            }
        }

        /// Whether this is ICU's root collation with its default
        /// attributes.
        pub fn is_root_default(&self) -> bool {
            if self.keywords || self.locale(false).as_deref() != Some("root") {
                return false;
            }
            let defaults = [
                (FRENCH_COLLATION, OFF),
                (ALTERNATE_HANDLING, NON_IGNORABLE),
                (CASE_FIRST, OFF),
                (CASE_LEVEL, OFF),
                (NORMALIZATION_MODE, OFF),
                (STRENGTH, TERTIARY),
                (HIRAGANA_QUATERNARY_MODE, OFF),
                (NUMERIC_COLLATION, OFF),
            ];
            // SAFETY: an open collator; asking for the reorder codes with no
            // buffer returns their number.
            unsafe {
                let mut status: UErrorCode = 0;
                let attrs = defaults.iter().all(|&(a, v)| (self.lib.get_attribute)(self.ptr, a, &mut status) == v);
                let mut st2: UErrorCode = 0;
                let reorder = (self.lib.get_reorder_codes)(self.ptr, std::ptr::null_mut(), 0, &mut st2);
                attrs
                    && status <= 0
                    && reorder == 0
                    && (self.lib.get_max_variable)(self.ptr) == REORDER_CODE_PUNCTUATION
            }
        }

        /// `ucol_strcollUTF8` (GNU R calls `ucol_strcollIter` over UTF-8
        /// iterators, the same comparison).
        #[inline(never)]
        pub fn cmp(&self, a: &str, b: &str) -> std::cmp::Ordering {
            // R strings are shorter than 2^31 bytes, and ICU fails only for
            // invalid arguments.
            let len = |s: &str| i32::try_from(s.len()).expect("could not collate using ICU");
            let (la, lb) = (len(a), len(b));
            let mut status: UErrorCode = 0;
            // SAFETY: an open collator and two UTF-8 buffers with their
            // lengths.
            let r =
                unsafe { (self.lib.strcoll_utf8)(self.ptr, a.as_ptr().cast(), la, b.as_ptr().cast(), lb, &mut status) };
            assert!(status <= 0, "could not collate using ICU");
            r.cmp(&0)
        }
    }

    #[cfg(test)]
    pub fn root() -> Option<Collator> {
        Collator::open_default("root").ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The ASCII table orders strings as ICU's root collator does.
    #[test]
    fn root_ascii_matches_icu() {
        let Some(coll) = icu::root() else {
            eprintln!("no ICU: skipped");
            return;
        };
        assert!(root_ascii_agrees(&coll));
        let check = |a: &str, b: &str| {
            if let Some(o) = root_ascii_cmp(a, b) {
                assert_eq!(o, coll.cmp(a, b), "{a:?} vs {b:?}");
            }
        };
        // Every pair of ASCII characters.
        let all: Vec<String> = (0..128u8).map(|c| (c as char).to_string()).collect();
        for a in &all {
            for b in &all {
                check(a, b);
            }
        }
        // Every pair of strings of up to two characters of an alphabet with
        // each kind of character.
        let alphabet = [
            "a", "A", "b", "B", "z", "Z", "0", "9", " ", "_", "-", ".", "$", "~", "\t", "\n", "\r", "\u{b}", "\u{c}",
            "\u{1}", "\u{7f}", "é", "l", "L", "4", "·", "\u{387}", "\u{fdd1}", "\u{301}",
        ];
        let mut strs = vec![String::new()];
        for a in alphabet {
            strs.push(a.to_string());
            for b in alphabet {
                strs.push(format!("{a}{b}"));
            }
        }
        for a in &strs {
            for b in &strs {
                check(a, b);
            }
        }
        // Every pair of strings of three characters around the contractions.
        let alphabet = ["l", "L", "a", "4", " ", "\u{1}", "·", "\u{fdd1}", "é", "\u{301}"];
        let mut strs = Vec::new();
        for a in alphabet {
            for b in alphabet {
                for c in alphabet {
                    strs.push(format!("{a}{b}{c}"));
                }
            }
        }
        for a in &strs {
            for b in &strs {
                check(a, b);
            }
        }
        // Longer printable strings.
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut next = |n: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed % n
        };
        for _ in 0..20000 {
            let mut word = || (0..next(7)).map(|_| (32 + next(95) as u8) as char).collect::<String>();
            let (a, b) = (word(), word());
            let b = if next(2) == 0 { format!("{}{}", &a[..a.len() / 2], b) } else { b };
            check(&a, &b);
        }
    }
}
