# needs: bayesim
# The user's own package (github.com/sims1253/bayesim): a simulation
# framework. Exercises that the package loads and its core config API
# is present, no Stan backend and no model fit.
suppressPackageStartupMessages(library(bayesim))
print(is.function(simulation_config))
print(is.function(run_simulation))
