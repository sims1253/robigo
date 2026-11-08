# needs: dagriculture
# The user's own package (github.com/sims1253/dagriculture), not in
# the pinned tree: state "absent" where it is not installed. This is
# its README example: registry, graph building, a gate, planning.
suppressPackageStartupMessages(library(dagriculture))
registry <- dagri_registry(
  dagri_kind("data_source"),
  dagri_kind("transform")
)
graph <- dagri_graph(registry) |>
  dagri_add_node("raw_data", "data_source") |>
  dagri_add_node("cleaned_data", "transform") |>
  dagri_add_edge(from = "raw_data", to = "cleaned_data", id = "edge_1")
graph <- dagri_add_gate(graph, edge = "edge_1", id = "review_gate")
graph <- dagri_recompute_state(graph)
print(dagri_eligible(graph))
print(dagri_blocked(graph))
print(dagri_topo_order(graph))
