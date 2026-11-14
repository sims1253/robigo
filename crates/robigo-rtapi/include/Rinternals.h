/* robigo's Rinternals.h (docs/g3.md §11): GNU R 4.6.2's own, then GNU R's
 * per-session C globals as macros over the isolate's block (robigo_capi.h):
 * in robigo they are per isolate, reached through the C API's one
 * thread-local. Package code built against robigo puts this directory
 * before GNU R's src/include (robigo-rtapi's tests and
 * scripts/rt-capi-build.sh do). */
#ifndef ROBIGO_RINTERNALS_H_
#define ROBIGO_RINTERNALS_H_

#include_next <Rinternals.h>
#include <robigo_capi.h>

/* (after GNU R's declarations, which these names must not reach) */
#define R_NilValue (robigo_cg->R_NilValue)
#define R_GlobalEnv (robigo_cg->R_GlobalEnv)
#define R_EmptyEnv (robigo_cg->R_EmptyEnv)
#define R_BaseEnv (robigo_cg->R_BaseEnv)
#define R_BaseNamespace (robigo_cg->R_BaseNamespace)
#define R_UnboundValue (robigo_cg->R_UnboundValue)
#define R_MissingArg (robigo_cg->R_MissingArg)
#define R_NaString (robigo_cg->R_NaString)
#define R_BlankString (robigo_cg->R_BlankString)
#define R_BlankScalarString (robigo_cg->R_BlankScalarString)
#define R_AsCharacterSymbol (robigo_cg->R_AsCharacterSymbol)
#define R_AtsignSymbol (robigo_cg->R_AtsignSymbol)
#define R_baseSymbol (robigo_cg->R_baseSymbol)
#define R_BaseSymbol (robigo_cg->R_BaseSymbol)
#define R_BraceSymbol (robigo_cg->R_BraceSymbol)
#define R_Bracket2Symbol (robigo_cg->R_Bracket2Symbol)
#define R_BracketSymbol (robigo_cg->R_BracketSymbol)
#define R_ClassSymbol (robigo_cg->R_ClassSymbol)
#define R_DeviceSymbol (robigo_cg->R_DeviceSymbol)
#define R_DimNamesSymbol (robigo_cg->R_DimNamesSymbol)
#define R_DimSymbol (robigo_cg->R_DimSymbol)
#define R_DollarSymbol (robigo_cg->R_DollarSymbol)
#define R_DotsSymbol (robigo_cg->R_DotsSymbol)
#define R_DoubleColonSymbol (robigo_cg->R_DoubleColonSymbol)
#define R_DropSymbol (robigo_cg->R_DropSymbol)
#define R_EvalSymbol (robigo_cg->R_EvalSymbol)
#define R_FunctionSymbol (robigo_cg->R_FunctionSymbol)
#define R_LastvalueSymbol (robigo_cg->R_LastvalueSymbol)
#define R_LevelsSymbol (robigo_cg->R_LevelsSymbol)
#define R_ModeSymbol (robigo_cg->R_ModeSymbol)
#define R_NaRmSymbol (robigo_cg->R_NaRmSymbol)
#define R_NameSymbol (robigo_cg->R_NameSymbol)
#define R_NamesSymbol (robigo_cg->R_NamesSymbol)
#define R_NamespaceEnvSymbol (robigo_cg->R_NamespaceEnvSymbol)
#define R_PackageSymbol (robigo_cg->R_PackageSymbol)
#define R_PreviousSymbol (robigo_cg->R_PreviousSymbol)
#define R_QuoteSymbol (robigo_cg->R_QuoteSymbol)
#define R_RowNamesSymbol (robigo_cg->R_RowNamesSymbol)
#define R_SeedsSymbol (robigo_cg->R_SeedsSymbol)
#define R_SortListSymbol (robigo_cg->R_SortListSymbol)
#define R_SourceSymbol (robigo_cg->R_SourceSymbol)
#define R_SpecSymbol (robigo_cg->R_SpecSymbol)
#define R_TripleColonSymbol (robigo_cg->R_TripleColonSymbol)
#define R_TspSymbol (robigo_cg->R_TspSymbol)
#define R_dot_defined (robigo_cg->R_dot_defined)
#define R_dot_Method (robigo_cg->R_dot_Method)
#define R_dot_packageName (robigo_cg->R_dot_packageName)
#define R_dot_target (robigo_cg->R_dot_target)
#define R_dot_Generic (robigo_cg->R_dot_Generic)

#endif /* ROBIGO_RINTERNALS_H_ */
