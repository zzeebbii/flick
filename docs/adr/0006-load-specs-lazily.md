# Load Specs lazily, one CLI at a time

The converted Specs come to 80 MB of JSON. Loading every one of them into typed structs measured 169 MB of memory. Loading the 11 CLIs the user runs most measured +2.7 MB, and loading one Spec takes about 1 ms (git: 0.7 ms). So the background process keeps only an index of CLI names in memory, loads a CLI's Spec the first time that CLI is typed, and evicts Specs that have not been used for a while. This is what keeps the memory budget from growing with the number of Specs shipped.
