# needs: bayeshear
# Excluded from the own-package block suite: imports brms (hard) and
# its tests fit/compare models, so the dependency tree and fitting are
# impractical in CI. Carried here as absent until that changes.
suppressPackageStartupMessages(library(bayeshear))
