//! Native routines of the grDevices package (src/library/grDevices/src).

use super::NativeFn;
use crate::error::{Flow, RResult};
use crate::interp::Interp;
use crate::sym::s;
use crate::value::*;

pub static PORTS: &[(&str, NativeFn)] = &[("cairoProps", cairo_props), ("colors", colors), ("col2rgb", col2rgb)];

/// Whether cairo (1) or pango-cairo (2) support was built in: robigo has no
/// cairo graphics devices.
// R: grDevices/src/init.c:cairoProps
fn cairo_props(_: &mut Interp, _: &Value, args: Vec<Arg>, _: &crate::env::Env) -> RResult<Value> {
    Ok(match crate::coerce::as_integer(&args[0].value) {
        1 | 2 => Value::lgl(false),
        _ => Value::Null,
    })
}

/// `colors()`: the names of the color database.
// R: grDevices/src/colors.c:colors
fn colors(_: &mut Interp, _: &Value, _: Vec<Arg>, _: &crate::env::Env) -> RResult<Value> {
    Ok(Value::strs_from(COLOR_DATABASE.iter().map(|&(n, _)| n)))
}

/// GNU R's color database: name and packed code `R_RGBA(r,g,b,a)`
/// (R: grDevices/src/colors.c:ColorDataBase).
static COLOR_DATABASE: &[(&str, u32)] = &[
    ("white", 0xffffffff),
    ("aliceblue", 0xfffff8f0),
    ("antiquewhite", 0xffd7ebfa),
    ("antiquewhite1", 0xffdbefff),
    ("antiquewhite2", 0xffccdfee),
    ("antiquewhite3", 0xffb0c0cd),
    ("antiquewhite4", 0xff78838b),
    ("aquamarine", 0xffd4ff7f),
    ("aquamarine1", 0xffd4ff7f),
    ("aquamarine2", 0xffc6ee76),
    ("aquamarine3", 0xffaacd66),
    ("aquamarine4", 0xff748b45),
    ("azure", 0xfffffff0),
    ("azure1", 0xfffffff0),
    ("azure2", 0xffeeeee0),
    ("azure3", 0xffcdcdc1),
    ("azure4", 0xff8b8b83),
    ("beige", 0xffdcf5f5),
    ("bisque", 0xffc4e4ff),
    ("bisque1", 0xffc4e4ff),
    ("bisque2", 0xffb7d5ee),
    ("bisque3", 0xff9eb7cd),
    ("bisque4", 0xff6b7d8b),
    ("black", 0xff000000),
    ("blanchedalmond", 0xffcdebff),
    ("blue", 0xffff0000),
    ("blue1", 0xffff0000),
    ("blue2", 0xffee0000),
    ("blue3", 0xffcd0000),
    ("blue4", 0xff8b0000),
    ("blueviolet", 0xffe22b8a),
    ("brown", 0xff2a2aa5),
    ("brown1", 0xff4040ff),
    ("brown2", 0xff3b3bee),
    ("brown3", 0xff3333cd),
    ("brown4", 0xff23238b),
    ("burlywood", 0xff87b8de),
    ("burlywood1", 0xff9bd3ff),
    ("burlywood2", 0xff91c5ee),
    ("burlywood3", 0xff7daacd),
    ("burlywood4", 0xff55738b),
    ("cadetblue", 0xffa09e5f),
    ("cadetblue1", 0xfffff598),
    ("cadetblue2", 0xffeee58e),
    ("cadetblue3", 0xffcdc57a),
    ("cadetblue4", 0xff8b8653),
    ("chartreuse", 0xff00ff7f),
    ("chartreuse1", 0xff00ff7f),
    ("chartreuse2", 0xff00ee76),
    ("chartreuse3", 0xff00cd66),
    ("chartreuse4", 0xff008b45),
    ("chocolate", 0xff1e69d2),
    ("chocolate1", 0xff247fff),
    ("chocolate2", 0xff2176ee),
    ("chocolate3", 0xff1d66cd),
    ("chocolate4", 0xff13458b),
    ("coral", 0xff507fff),
    ("coral1", 0xff5672ff),
    ("coral2", 0xff506aee),
    ("coral3", 0xff455bcd),
    ("coral4", 0xff2f3e8b),
    ("cornflowerblue", 0xffed9564),
    ("cornsilk", 0xffdcf8ff),
    ("cornsilk1", 0xffdcf8ff),
    ("cornsilk2", 0xffcde8ee),
    ("cornsilk3", 0xffb1c8cd),
    ("cornsilk4", 0xff78888b),
    ("cyan", 0xffffff00),
    ("cyan1", 0xffffff00),
    ("cyan2", 0xffeeee00),
    ("cyan3", 0xffcdcd00),
    ("cyan4", 0xff8b8b00),
    ("darkblue", 0xff8b0000),
    ("darkcyan", 0xff8b8b00),
    ("darkgoldenrod", 0xff0b86b8),
    ("darkgoldenrod1", 0xff0fb9ff),
    ("darkgoldenrod2", 0xff0eadee),
    ("darkgoldenrod3", 0xff0c95cd),
    ("darkgoldenrod4", 0xff08658b),
    ("darkgray", 0xffa9a9a9),
    ("darkgreen", 0xff006400),
    ("darkgrey", 0xffa9a9a9),
    ("darkkhaki", 0xff6bb7bd),
    ("darkmagenta", 0xff8b008b),
    ("darkolivegreen", 0xff2f6b55),
    ("darkolivegreen1", 0xff70ffca),
    ("darkolivegreen2", 0xff68eebc),
    ("darkolivegreen3", 0xff5acda2),
    ("darkolivegreen4", 0xff3d8b6e),
    ("darkorange", 0xff008cff),
    ("darkorange1", 0xff007fff),
    ("darkorange2", 0xff0076ee),
    ("darkorange3", 0xff0066cd),
    ("darkorange4", 0xff00458b),
    ("darkorchid", 0xffcc3299),
    ("darkorchid1", 0xffff3ebf),
    ("darkorchid2", 0xffee3ab2),
    ("darkorchid3", 0xffcd329a),
    ("darkorchid4", 0xff8b2268),
    ("darkred", 0xff00008b),
    ("darksalmon", 0xff7a96e9),
    ("darkseagreen", 0xff8fbc8f),
    ("darkseagreen1", 0xffc1ffc1),
    ("darkseagreen2", 0xffb4eeb4),
    ("darkseagreen3", 0xff9bcd9b),
    ("darkseagreen4", 0xff698b69),
    ("darkslateblue", 0xff8b3d48),
    ("darkslategray", 0xff4f4f2f),
    ("darkslategray1", 0xffffff97),
    ("darkslategray2", 0xffeeee8d),
    ("darkslategray3", 0xffcdcd79),
    ("darkslategray4", 0xff8b8b52),
    ("darkslategrey", 0xff4f4f2f),
    ("darkturquoise", 0xffd1ce00),
    ("darkviolet", 0xffd30094),
    ("deeppink", 0xff9314ff),
    ("deeppink1", 0xff9314ff),
    ("deeppink2", 0xff8912ee),
    ("deeppink3", 0xff7610cd),
    ("deeppink4", 0xff500a8b),
    ("deepskyblue", 0xffffbf00),
    ("deepskyblue1", 0xffffbf00),
    ("deepskyblue2", 0xffeeb200),
    ("deepskyblue3", 0xffcd9a00),
    ("deepskyblue4", 0xff8b6800),
    ("dimgray", 0xff696969),
    ("dimgrey", 0xff696969),
    ("dodgerblue", 0xffff901e),
    ("dodgerblue1", 0xffff901e),
    ("dodgerblue2", 0xffee861c),
    ("dodgerblue3", 0xffcd7418),
    ("dodgerblue4", 0xff8b4e10),
    ("firebrick", 0xff2222b2),
    ("firebrick1", 0xff3030ff),
    ("firebrick2", 0xff2c2cee),
    ("firebrick3", 0xff2626cd),
    ("firebrick4", 0xff1a1a8b),
    ("floralwhite", 0xfff0faff),
    ("forestgreen", 0xff228b22),
    ("gainsboro", 0xffdcdcdc),
    ("ghostwhite", 0xfffff8f8),
    ("gold", 0xff00d7ff),
    ("gold1", 0xff00d7ff),
    ("gold2", 0xff00c9ee),
    ("gold3", 0xff00adcd),
    ("gold4", 0xff00758b),
    ("goldenrod", 0xff20a5da),
    ("goldenrod1", 0xff25c1ff),
    ("goldenrod2", 0xff22b4ee),
    ("goldenrod3", 0xff1d9bcd),
    ("goldenrod4", 0xff14698b),
    ("gray", 0xffbebebe),
    ("gray0", 0xff000000),
    ("gray1", 0xff030303),
    ("gray2", 0xff050505),
    ("gray3", 0xff080808),
    ("gray4", 0xff0a0a0a),
    ("gray5", 0xff0d0d0d),
    ("gray6", 0xff0f0f0f),
    ("gray7", 0xff121212),
    ("gray8", 0xff141414),
    ("gray9", 0xff171717),
    ("gray10", 0xff1a1a1a),
    ("gray11", 0xff1c1c1c),
    ("gray12", 0xff1f1f1f),
    ("gray13", 0xff212121),
    ("gray14", 0xff242424),
    ("gray15", 0xff262626),
    ("gray16", 0xff292929),
    ("gray17", 0xff2b2b2b),
    ("gray18", 0xff2e2e2e),
    ("gray19", 0xff303030),
    ("gray20", 0xff333333),
    ("gray21", 0xff363636),
    ("gray22", 0xff383838),
    ("gray23", 0xff3b3b3b),
    ("gray24", 0xff3d3d3d),
    ("gray25", 0xff404040),
    ("gray26", 0xff424242),
    ("gray27", 0xff454545),
    ("gray28", 0xff474747),
    ("gray29", 0xff4a4a4a),
    ("gray30", 0xff4d4d4d),
    ("gray31", 0xff4f4f4f),
    ("gray32", 0xff525252),
    ("gray33", 0xff545454),
    ("gray34", 0xff575757),
    ("gray35", 0xff595959),
    ("gray36", 0xff5c5c5c),
    ("gray37", 0xff5e5e5e),
    ("gray38", 0xff616161),
    ("gray39", 0xff636363),
    ("gray40", 0xff666666),
    ("gray41", 0xff696969),
    ("gray42", 0xff6b6b6b),
    ("gray43", 0xff6e6e6e),
    ("gray44", 0xff707070),
    ("gray45", 0xff737373),
    ("gray46", 0xff757575),
    ("gray47", 0xff787878),
    ("gray48", 0xff7a7a7a),
    ("gray49", 0xff7d7d7d),
    ("gray50", 0xff7f7f7f),
    ("gray51", 0xff828282),
    ("gray52", 0xff858585),
    ("gray53", 0xff878787),
    ("gray54", 0xff8a8a8a),
    ("gray55", 0xff8c8c8c),
    ("gray56", 0xff8f8f8f),
    ("gray57", 0xff919191),
    ("gray58", 0xff949494),
    ("gray59", 0xff969696),
    ("gray60", 0xff999999),
    ("gray61", 0xff9c9c9c),
    ("gray62", 0xff9e9e9e),
    ("gray63", 0xffa1a1a1),
    ("gray64", 0xffa3a3a3),
    ("gray65", 0xffa6a6a6),
    ("gray66", 0xffa8a8a8),
    ("gray67", 0xffababab),
    ("gray68", 0xffadadad),
    ("gray69", 0xffb0b0b0),
    ("gray70", 0xffb3b3b3),
    ("gray71", 0xffb5b5b5),
    ("gray72", 0xffb8b8b8),
    ("gray73", 0xffbababa),
    ("gray74", 0xffbdbdbd),
    ("gray75", 0xffbfbfbf),
    ("gray76", 0xffc2c2c2),
    ("gray77", 0xffc4c4c4),
    ("gray78", 0xffc7c7c7),
    ("gray79", 0xffc9c9c9),
    ("gray80", 0xffcccccc),
    ("gray81", 0xffcfcfcf),
    ("gray82", 0xffd1d1d1),
    ("gray83", 0xffd4d4d4),
    ("gray84", 0xffd6d6d6),
    ("gray85", 0xffd9d9d9),
    ("gray86", 0xffdbdbdb),
    ("gray87", 0xffdedede),
    ("gray88", 0xffe0e0e0),
    ("gray89", 0xffe3e3e3),
    ("gray90", 0xffe5e5e5),
    ("gray91", 0xffe8e8e8),
    ("gray92", 0xffebebeb),
    ("gray93", 0xffededed),
    ("gray94", 0xfff0f0f0),
    ("gray95", 0xfff2f2f2),
    ("gray96", 0xfff5f5f5),
    ("gray97", 0xfff7f7f7),
    ("gray98", 0xfffafafa),
    ("gray99", 0xfffcfcfc),
    ("gray100", 0xffffffff),
    ("green", 0xff00ff00),
    ("green1", 0xff00ff00),
    ("green2", 0xff00ee00),
    ("green3", 0xff00cd00),
    ("green4", 0xff008b00),
    ("greenyellow", 0xff2fffad),
    ("grey", 0xffbebebe),
    ("grey0", 0xff000000),
    ("grey1", 0xff030303),
    ("grey2", 0xff050505),
    ("grey3", 0xff080808),
    ("grey4", 0xff0a0a0a),
    ("grey5", 0xff0d0d0d),
    ("grey6", 0xff0f0f0f),
    ("grey7", 0xff121212),
    ("grey8", 0xff141414),
    ("grey9", 0xff171717),
    ("grey10", 0xff1a1a1a),
    ("grey11", 0xff1c1c1c),
    ("grey12", 0xff1f1f1f),
    ("grey13", 0xff212121),
    ("grey14", 0xff242424),
    ("grey15", 0xff262626),
    ("grey16", 0xff292929),
    ("grey17", 0xff2b2b2b),
    ("grey18", 0xff2e2e2e),
    ("grey19", 0xff303030),
    ("grey20", 0xff333333),
    ("grey21", 0xff363636),
    ("grey22", 0xff383838),
    ("grey23", 0xff3b3b3b),
    ("grey24", 0xff3d3d3d),
    ("grey25", 0xff404040),
    ("grey26", 0xff424242),
    ("grey27", 0xff454545),
    ("grey28", 0xff474747),
    ("grey29", 0xff4a4a4a),
    ("grey30", 0xff4d4d4d),
    ("grey31", 0xff4f4f4f),
    ("grey32", 0xff525252),
    ("grey33", 0xff545454),
    ("grey34", 0xff575757),
    ("grey35", 0xff595959),
    ("grey36", 0xff5c5c5c),
    ("grey37", 0xff5e5e5e),
    ("grey38", 0xff616161),
    ("grey39", 0xff636363),
    ("grey40", 0xff666666),
    ("grey41", 0xff696969),
    ("grey42", 0xff6b6b6b),
    ("grey43", 0xff6e6e6e),
    ("grey44", 0xff707070),
    ("grey45", 0xff737373),
    ("grey46", 0xff757575),
    ("grey47", 0xff787878),
    ("grey48", 0xff7a7a7a),
    ("grey49", 0xff7d7d7d),
    ("grey50", 0xff7f7f7f),
    ("grey51", 0xff828282),
    ("grey52", 0xff858585),
    ("grey53", 0xff878787),
    ("grey54", 0xff8a8a8a),
    ("grey55", 0xff8c8c8c),
    ("grey56", 0xff8f8f8f),
    ("grey57", 0xff919191),
    ("grey58", 0xff949494),
    ("grey59", 0xff969696),
    ("grey60", 0xff999999),
    ("grey61", 0xff9c9c9c),
    ("grey62", 0xff9e9e9e),
    ("grey63", 0xffa1a1a1),
    ("grey64", 0xffa3a3a3),
    ("grey65", 0xffa6a6a6),
    ("grey66", 0xffa8a8a8),
    ("grey67", 0xffababab),
    ("grey68", 0xffadadad),
    ("grey69", 0xffb0b0b0),
    ("grey70", 0xffb3b3b3),
    ("grey71", 0xffb5b5b5),
    ("grey72", 0xffb8b8b8),
    ("grey73", 0xffbababa),
    ("grey74", 0xffbdbdbd),
    ("grey75", 0xffbfbfbf),
    ("grey76", 0xffc2c2c2),
    ("grey77", 0xffc4c4c4),
    ("grey78", 0xffc7c7c7),
    ("grey79", 0xffc9c9c9),
    ("grey80", 0xffcccccc),
    ("grey81", 0xffcfcfcf),
    ("grey82", 0xffd1d1d1),
    ("grey83", 0xffd4d4d4),
    ("grey84", 0xffd6d6d6),
    ("grey85", 0xffd9d9d9),
    ("grey86", 0xffdbdbdb),
    ("grey87", 0xffdedede),
    ("grey88", 0xffe0e0e0),
    ("grey89", 0xffe3e3e3),
    ("grey90", 0xffe5e5e5),
    ("grey91", 0xffe8e8e8),
    ("grey92", 0xffebebeb),
    ("grey93", 0xffededed),
    ("grey94", 0xfff0f0f0),
    ("grey95", 0xfff2f2f2),
    ("grey96", 0xfff5f5f5),
    ("grey97", 0xfff7f7f7),
    ("grey98", 0xfffafafa),
    ("grey99", 0xfffcfcfc),
    ("grey100", 0xffffffff),
    ("honeydew", 0xfff0fff0),
    ("honeydew1", 0xfff0fff0),
    ("honeydew2", 0xffe0eee0),
    ("honeydew3", 0xffc1cdc1),
    ("honeydew4", 0xff838b83),
    ("hotpink", 0xffb469ff),
    ("hotpink1", 0xffb46eff),
    ("hotpink2", 0xffa76aee),
    ("hotpink3", 0xff9060cd),
    ("hotpink4", 0xff623a8b),
    ("indianred", 0xff5c5ccd),
    ("indianred1", 0xff6a6aff),
    ("indianred2", 0xff6363ee),
    ("indianred3", 0xff5555cd),
    ("indianred4", 0xff3a3a8b),
    ("ivory", 0xfff0ffff),
    ("ivory1", 0xfff0ffff),
    ("ivory2", 0xffe0eeee),
    ("ivory3", 0xffc1cdcd),
    ("ivory4", 0xff838b8b),
    ("khaki", 0xff8ce6f0),
    ("khaki1", 0xff8ff6ff),
    ("khaki2", 0xff85e6ee),
    ("khaki3", 0xff73c6cd),
    ("khaki4", 0xff4e868b),
    ("lavender", 0xfffae6e6),
    ("lavenderblush", 0xfff5f0ff),
    ("lavenderblush1", 0xfff5f0ff),
    ("lavenderblush2", 0xffe5e0ee),
    ("lavenderblush3", 0xffc5c1cd),
    ("lavenderblush4", 0xff86838b),
    ("lawngreen", 0xff00fc7c),
    ("lemonchiffon", 0xffcdfaff),
    ("lemonchiffon1", 0xffcdfaff),
    ("lemonchiffon2", 0xffbfe9ee),
    ("lemonchiffon3", 0xffa5c9cd),
    ("lemonchiffon4", 0xff70898b),
    ("lightblue", 0xffe6d8ad),
    ("lightblue1", 0xffffefbf),
    ("lightblue2", 0xffeedfb2),
    ("lightblue3", 0xffcdc09a),
    ("lightblue4", 0xff8b8368),
    ("lightcoral", 0xff8080f0),
    ("lightcyan", 0xffffffe0),
    ("lightcyan1", 0xffffffe0),
    ("lightcyan2", 0xffeeeed1),
    ("lightcyan3", 0xffcdcdb4),
    ("lightcyan4", 0xff8b8b7a),
    ("lightgoldenrod", 0xff82ddee),
    ("lightgoldenrod1", 0xff8becff),
    ("lightgoldenrod2", 0xff82dcee),
    ("lightgoldenrod3", 0xff70becd),
    ("lightgoldenrod4", 0xff4c818b),
    ("lightgoldenrodyellow", 0xffd2fafa),
    ("lightgray", 0xffd3d3d3),
    ("lightgreen", 0xff90ee90),
    ("lightgrey", 0xffd3d3d3),
    ("lightpink", 0xffc1b6ff),
    ("lightpink1", 0xffb9aeff),
    ("lightpink2", 0xffada2ee),
    ("lightpink3", 0xff958ccd),
    ("lightpink4", 0xff655f8b),
    ("lightsalmon", 0xff7aa0ff),
    ("lightsalmon1", 0xff7aa0ff),
    ("lightsalmon2", 0xff7295ee),
    ("lightsalmon3", 0xff6281cd),
    ("lightsalmon4", 0xff42578b),
    ("lightseagreen", 0xffaab220),
    ("lightskyblue", 0xffface87),
    ("lightskyblue1", 0xffffe2b0),
    ("lightskyblue2", 0xffeed3a4),
    ("lightskyblue3", 0xffcdb68d),
    ("lightskyblue4", 0xff8b7b60),
    ("lightslateblue", 0xffff7084),
    ("lightslategray", 0xff998877),
    ("lightslategrey", 0xff998877),
    ("lightsteelblue", 0xffdec4b0),
    ("lightsteelblue1", 0xffffe1ca),
    ("lightsteelblue2", 0xffeed2bc),
    ("lightsteelblue3", 0xffcdb5a2),
    ("lightsteelblue4", 0xff8b7b6e),
    ("lightyellow", 0xffe0ffff),
    ("lightyellow1", 0xffe0ffff),
    ("lightyellow2", 0xffd1eeee),
    ("lightyellow3", 0xffb4cdcd),
    ("lightyellow4", 0xff7a8b8b),
    ("limegreen", 0xff32cd32),
    ("linen", 0xffe6f0fa),
    ("magenta", 0xffff00ff),
    ("magenta1", 0xffff00ff),
    ("magenta2", 0xffee00ee),
    ("magenta3", 0xffcd00cd),
    ("magenta4", 0xff8b008b),
    ("maroon", 0xff6030b0),
    ("maroon1", 0xffb334ff),
    ("maroon2", 0xffa730ee),
    ("maroon3", 0xff9029cd),
    ("maroon4", 0xff621c8b),
    ("mediumaquamarine", 0xffaacd66),
    ("mediumblue", 0xffcd0000),
    ("mediumorchid", 0xffd355ba),
    ("mediumorchid1", 0xffff66e0),
    ("mediumorchid2", 0xffee5fd1),
    ("mediumorchid3", 0xffcd52b4),
    ("mediumorchid4", 0xff8b377a),
    ("mediumpurple", 0xffdb7093),
    ("mediumpurple1", 0xffff82ab),
    ("mediumpurple2", 0xffee799f),
    ("mediumpurple3", 0xffcd6889),
    ("mediumpurple4", 0xff8b475d),
    ("mediumseagreen", 0xff71b33c),
    ("mediumslateblue", 0xffee687b),
    ("mediumspringgreen", 0xff9afa00),
    ("mediumturquoise", 0xffccd148),
    ("mediumvioletred", 0xff8515c7),
    ("midnightblue", 0xff701919),
    ("mintcream", 0xfffafff5),
    ("mistyrose", 0xffe1e4ff),
    ("mistyrose1", 0xffe1e4ff),
    ("mistyrose2", 0xffd2d5ee),
    ("mistyrose3", 0xffb5b7cd),
    ("mistyrose4", 0xff7b7d8b),
    ("moccasin", 0xffb5e4ff),
    ("navajowhite", 0xffaddeff),
    ("navajowhite1", 0xffaddeff),
    ("navajowhite2", 0xffa1cfee),
    ("navajowhite3", 0xff8bb3cd),
    ("navajowhite4", 0xff5e798b),
    ("navy", 0xff800000),
    ("navyblue", 0xff800000),
    ("oldlace", 0xffe6f5fd),
    ("olivedrab", 0xff238e6b),
    ("olivedrab1", 0xff3effc0),
    ("olivedrab2", 0xff3aeeb3),
    ("olivedrab3", 0xff32cd9a),
    ("olivedrab4", 0xff228b69),
    ("orange", 0xff00a5ff),
    ("orange1", 0xff00a5ff),
    ("orange2", 0xff009aee),
    ("orange3", 0xff0085cd),
    ("orange4", 0xff005a8b),
    ("orangered", 0xff0045ff),
    ("orangered1", 0xff0045ff),
    ("orangered2", 0xff0040ee),
    ("orangered3", 0xff0037cd),
    ("orangered4", 0xff00258b),
    ("orchid", 0xffd670da),
    ("orchid1", 0xfffa83ff),
    ("orchid2", 0xffe97aee),
    ("orchid3", 0xffc969cd),
    ("orchid4", 0xff89478b),
    ("palegoldenrod", 0xffaae8ee),
    ("palegreen", 0xff98fb98),
    ("palegreen1", 0xff9aff9a),
    ("palegreen2", 0xff90ee90),
    ("palegreen3", 0xff7ccd7c),
    ("palegreen4", 0xff548b54),
    ("paleturquoise", 0xffeeeeaf),
    ("paleturquoise1", 0xffffffbb),
    ("paleturquoise2", 0xffeeeeae),
    ("paleturquoise3", 0xffcdcd96),
    ("paleturquoise4", 0xff8b8b66),
    ("palevioletred", 0xff9370db),
    ("palevioletred1", 0xffab82ff),
    ("palevioletred2", 0xff9f79ee),
    ("palevioletred3", 0xff8968cd),
    ("palevioletred4", 0xff5d478b),
    ("papayawhip", 0xffd5efff),
    ("peachpuff", 0xffb9daff),
    ("peachpuff1", 0xffb9daff),
    ("peachpuff2", 0xffadcbee),
    ("peachpuff3", 0xff95afcd),
    ("peachpuff4", 0xff65778b),
    ("peru", 0xff3f85cd),
    ("pink", 0xffcbc0ff),
    ("pink1", 0xffc5b5ff),
    ("pink2", 0xffb8a9ee),
    ("pink3", 0xff9e91cd),
    ("pink4", 0xff6c638b),
    ("plum", 0xffdda0dd),
    ("plum1", 0xffffbbff),
    ("plum2", 0xffeeaeee),
    ("plum3", 0xffcd96cd),
    ("plum4", 0xff8b668b),
    ("powderblue", 0xffe6e0b0),
    ("purple", 0xfff020a0),
    ("purple1", 0xffff309b),
    ("purple2", 0xffee2c91),
    ("purple3", 0xffcd267d),
    ("purple4", 0xff8b1a55),
    ("red", 0xff0000ff),
    ("red1", 0xff0000ff),
    ("red2", 0xff0000ee),
    ("red3", 0xff0000cd),
    ("red4", 0xff00008b),
    ("rosybrown", 0xff8f8fbc),
    ("rosybrown1", 0xffc1c1ff),
    ("rosybrown2", 0xffb4b4ee),
    ("rosybrown3", 0xff9b9bcd),
    ("rosybrown4", 0xff69698b),
    ("royalblue", 0xffe16941),
    ("royalblue1", 0xffff7648),
    ("royalblue2", 0xffee6e43),
    ("royalblue3", 0xffcd5f3a),
    ("royalblue4", 0xff8b4027),
    ("saddlebrown", 0xff13458b),
    ("salmon", 0xff7280fa),
    ("salmon1", 0xff698cff),
    ("salmon2", 0xff6282ee),
    ("salmon3", 0xff5470cd),
    ("salmon4", 0xff394c8b),
    ("sandybrown", 0xff60a4f4),
    ("seagreen", 0xff578b2e),
    ("seagreen1", 0xff9fff54),
    ("seagreen2", 0xff94ee4e),
    ("seagreen3", 0xff80cd43),
    ("seagreen4", 0xff578b2e),
    ("seashell", 0xffeef5ff),
    ("seashell1", 0xffeef5ff),
    ("seashell2", 0xffdee5ee),
    ("seashell3", 0xffbfc5cd),
    ("seashell4", 0xff82868b),
    ("sienna", 0xff2d52a0),
    ("sienna1", 0xff4782ff),
    ("sienna2", 0xff4279ee),
    ("sienna3", 0xff3968cd),
    ("sienna4", 0xff26478b),
    ("skyblue", 0xffebce87),
    ("skyblue1", 0xffffce87),
    ("skyblue2", 0xffeec07e),
    ("skyblue3", 0xffcda66c),
    ("skyblue4", 0xff8b704a),
    ("slateblue", 0xffcd5a6a),
    ("slateblue1", 0xffff6f83),
    ("slateblue2", 0xffee677a),
    ("slateblue3", 0xffcd5969),
    ("slateblue4", 0xff8b3c47),
    ("slategray", 0xff908070),
    ("slategray1", 0xffffe2c6),
    ("slategray2", 0xffeed3b9),
    ("slategray3", 0xffcdb69f),
    ("slategray4", 0xff8b7b6c),
    ("slategrey", 0xff908070),
    ("snow", 0xfffafaff),
    ("snow1", 0xfffafaff),
    ("snow2", 0xffe9e9ee),
    ("snow3", 0xffc9c9cd),
    ("snow4", 0xff89898b),
    ("springgreen", 0xff7fff00),
    ("springgreen1", 0xff7fff00),
    ("springgreen2", 0xff76ee00),
    ("springgreen3", 0xff66cd00),
    ("springgreen4", 0xff458b00),
    ("steelblue", 0xffb48246),
    ("steelblue1", 0xffffb863),
    ("steelblue2", 0xffeeac5c),
    ("steelblue3", 0xffcd944f),
    ("steelblue4", 0xff8b6436),
    ("tan", 0xff8cb4d2),
    ("tan1", 0xff4fa5ff),
    ("tan2", 0xff499aee),
    ("tan3", 0xff3f85cd),
    ("tan4", 0xff2b5a8b),
    ("thistle", 0xffd8bfd8),
    ("thistle1", 0xffffe1ff),
    ("thistle2", 0xffeed2ee),
    ("thistle3", 0xffcdb5cd),
    ("thistle4", 0xff8b7b8b),
    ("tomato", 0xff4763ff),
    ("tomato1", 0xff4763ff),
    ("tomato2", 0xff425cee),
    ("tomato3", 0xff394fcd),
    ("tomato4", 0xff26368b),
    ("turquoise", 0xffd0e040),
    ("turquoise1", 0xfffff500),
    ("turquoise2", 0xffeee500),
    ("turquoise3", 0xffcdc500),
    ("turquoise4", 0xff8b8600),
    ("violet", 0xffee82ee),
    ("violetred", 0xff9020d0),
    ("violetred1", 0xff963eff),
    ("violetred2", 0xff8c3aee),
    ("violetred3", 0xff7832cd),
    ("violetred4", 0xff52228b),
    ("wheat", 0xffb3def5),
    ("wheat1", 0xffbae7ff),
    ("wheat2", 0xffaed8ee),
    ("wheat3", 0xff96bacd),
    ("wheat4", 0xff667e8b),
    ("whitesmoke", 0xfff5f5f5),
    ("yellow", 0xff00ffff),
    ("yellow1", 0xff00ffff),
    ("yellow2", 0xff00eeee),
    ("yellow3", 0xff00cdcd),
    ("yellow4", 0xff008b8b),
    ("yellowgreen", 0xff32cd9a),
];

// ---- col2rgb (R: grDevices/src/colors.c) ------------------------------------

const R_TRANWHITE: u32 = 0x00ff_ffff; // R_RGBA(255, 255, 255, 0)
/// The default palette (R: colors.c:DefaultPalette); `palette()` is not
/// ported, so the palette cannot change.
static DEFAULT_PALETTE: [u32; 8] =
    [0xff00_0000, 0xff6b_53df, 0xff4f_d061, 0xffe6_9722, 0xffe5_e228, 0xffbc_0bcd, 0xff10_c7f5, 0xff9e_9e9e];

/// Case-insensitive compare that squeezes blanks out of both strings.
// R: colors.c:StrMatch
fn str_match(s: &str, t: &str) -> bool {
    let a: Vec<char> = s.chars().filter(|&c| c != ' ').flat_map(|c| c.to_lowercase()).collect();
    let b: Vec<char> = t.chars().filter(|&c| c != ' ').flat_map(|c| c.to_lowercase()).collect();
    a == b
}

/// Hex digit to integer.
// R: colors.c:hexdigit
fn hexdigit(d: u8) -> RResult<u32> {
    match d {
        b'0'..=b'9' => Ok((d - b'0') as u32),
        b'A'..=b'F' => Ok(10 + (d - b'A') as u32),
        b'a'..=b'f' => Ok(10 + (d - b'a') as u32),
        _ => Err(Flow::error("invalid hex digit in 'color' or 'lty'")),
    }
}

/// `#RRGGBB[AA]` or `#RGB[A]` to a packed color.
// R: colors.c:rgb2col
fn rgb2col(rgb: &str) -> RResult<u32> {
    if !rgb.starts_with('#') {
        return Err(Flow::error("invalid RGB specification"));
    }
    let b = rgb.as_bytes();
    let d = |i: usize| hexdigit(b[i]);
    let (mut r, mut g, mut bl, mut a) = (0u32, 0u32, 0u32, 0u32);
    match b.len() {
        9 => {
            a = 16 * d(7)? + d(8)?;
            (r, g, bl) = (16 * d(1)? + d(2)?, 16 * d(3)? + d(4)?, 16 * d(5)? + d(6)?);
        }
        7 => (r, g, bl) = (16 * d(1)? + d(2)?, 16 * d(3)? + d(4)?, 16 * d(5)? + d(6)?),
        5 => {
            a = 17 * d(4)?;
            (r, g, bl) = (17 * d(1)?, 17 * d(2)?, 17 * d(3)?);
        }
        4 => (r, g, bl) = (17 * d(1)?, 17 * d(2)?, 17 * d(3)?),
        _ => return Err(Flow::error("invalid RGB specification")),
    }
    #[allow(unused_assignments)]
    let _ = (r, g, bl, a);
    Ok(match b.len() {
        7 | 4 => r | (g << 8) | (bl << 16) | 0xff00_0000,
        _ => r | (g << 8) | (bl << 16) | (a << 24),
    })
}

/// A color name to a packed color.
// R: colors.c:name2col
fn name2col(nm: &str) -> RResult<u32> {
    if nm == "NA" || nm == "transparent" {
        return Ok(R_TRANWHITE);
    }
    for &(name, code) in COLOR_DATABASE {
        if str_match(name, nm) {
            return Ok(code);
        }
    }
    Err(Flow::error(format!("invalid color name '{nm}'")))
}

/// `strtod` with C's whole-string check, for palette indices.
fn strtod_index(s: &str) -> Option<f64> {
    let t = s.trim_start();
    let digits_end =
        t.find(|c: char| !(c.is_ascii_digit() || matches!(c, '.' | 'e' | 'E' | '+' | '-'))).unwrap_or(t.len());
    let (head, rest) = t.split_at(digits_end);
    if !rest.is_empty() || head.is_empty() {
        return None;
    }
    head.parse::<f64>().ok()
}

/// One color specification string.
// R: colors.c:str2col
fn str2col(s: &str, bg: u32) -> RResult<u32> {
    if s.as_bytes().first() == Some(&b'#') {
        rgb2col(s)
    } else if s.as_bytes().first().is_some_and(|c| c.is_ascii_digit()) {
        // R: colors.c:str2col truncates to int BEFORE the zero check,
        // so "0.5" is index 0 (the background color).
        let indx = match strtod_index(s) {
            Some(v) => v as i64,
            None => return Err(Flow::error(format!("invalid color specification \"{s}\""))),
        };
        if indx == 0 {
            return Ok(bg);
        }
        Ok(DEFAULT_PALETTE[((indx - 1) % 8) as usize])
    } else {
        name2col(s)
    }
}

/// `col2rgb(colors, alpha)`: the color table, names, hex strings and
/// palette indices as a red/green/blue[/alpha] integer matrix.
// R: grDevices/src/colors.c:col2rgb
fn col2rgb(i: &mut Interp, call: &Value, mut args: Vec<Arg>, _: &crate::env::Env) -> RResult<Value> {
    let alpha = crate::coerce::as_logical(&args.pop().unwrap().value);
    if alpha == NA_LOGICAL {
        return Err(Flow::error_call(call.clone(), "invalid 'alpha' value"));
    }
    let alph = alpha == 1;
    let colors = args.pop().unwrap().value;
    // REALSXP coerces to integer, anything else to character.
    let colors = match &colors {
        Value::Int(_) | Value::Str(_) => colors,
        Value::Real(_) => i.coerce(&colors, VType::Int)?,
        _ => i.coerce(&colors, VType::Str)?,
    };
    let names = crate::attrib::names_vec(&colors);
    let n = colors.len();
    let mut out = Vec::with_capacity(n * (3 + alph as usize));
    // inRGBpar3 with bg = R_TRANWHITE: character specs or integer indices.
    let ints = match &colors {
        Value::Int(v) => Some(v.data.clone()),
        _ => None,
    };
    for idx in 0..n {
        let icol = match (&colors, &ints) {
            (Value::Str(s), _) => {
                let spec = s.data[idx].clone();
                match spec {
                    Some(s) => str2col(&s, R_TRANWHITE)?,
                    // STRING_ELT NA: CHAR(NA) is "NA", which name2col maps
                    // to transparent white.
                    None => str2col("NA", R_TRANWHITE)?,
                }
            }
            (_, Some(data)) => {
                let indx = data.get(idx).copied().unwrap_or(NA_INTEGER);
                if indx == NA_INTEGER {
                    R_TRANWHITE
                } else if indx < 0 {
                    return Err(Flow::error(format!("numerical color values must be >= 0, found {indx}")));
                } else if indx == 0 {
                    R_TRANWHITE
                } else {
                    DEFAULT_PALETTE[((indx as i64 - 1) % 8) as usize]
                }
            }
            // inRGBpar3's default (unreachable after the coercions above):
            // warn and fall back to the background color.
            _ => {
                i.warning("supplied color is neither numeric nor character")?;
                R_TRANWHITE
            }
        };
        out.push((icol & 255) as i32);
        out.push(((icol >> 8) & 255) as i32);
        out.push(((icol >> 16) & 255) as i32);
        if alph {
            out.push(((icol >> 24) & 255) as i32);
        }
    }
    let mut ans = Value::ints(out);
    ans.set_attr_raw(s::DIM, Value::ints(vec![3 + alph as i32, n as i32]));
    let row_names: Vec<RStr> = if alph {
        vec![Some("red".into()), Some("green".into()), Some("blue".into()), Some("alpha".into())]
    } else {
        vec![Some("red".into()), Some("green".into()), Some("blue".into())]
    };
    let col_names = names.map(|n| Value::strs(n)).unwrap_or(Value::Null);
    ans.set_attr_raw(s::DIMNAMES, Value::list(vec![Value::strs(row_names), col_names]));
    Ok(ans)
}
