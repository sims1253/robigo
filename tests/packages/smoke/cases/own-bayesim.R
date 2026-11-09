# needs: bayesim
# The user's own package (github.com/sims1253/bayesim): simulation
# studies of Bayesian models. Its README study ("does a 90% interval
# cover the true slope?") with exact linear regression, no Stan, and
# 10 replicates instead of 100 to stay in seconds.
suppressPackageStartupMessages(library(bayesim))
gen <- function(data_spec, task_ctx) {
  x <- rnorm(data_spec$n)
  y <- 1 + 0.5 * x + rnorm(data_spec$n)
  list(
    train = data.frame(y = y, x = x),
    response = "y",
    true_params = c(x = 0.5),
    vars_of_interest = "x"
  )
}
config <- simulation_config(
  data_grid = data.frame(n = c(20, 50, 200)),
  fit_grid = data.frame(model = "linear"),
  data_generator = gen,
  fitter = LinearRegressionFitter(),
  metrics = list(coverage_metric(prob = 0.90)),
  n_replicates = 10L,
  seed = 42L
)
result <- run_simulation(config, progress = FALSE, verbose = FALSE)
summary <- summarize_simulation(result, by = "data_n", metrics = "coverage__by_param__x")
print(as.data.frame(summary))
