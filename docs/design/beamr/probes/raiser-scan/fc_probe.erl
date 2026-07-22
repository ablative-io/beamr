-module(fc_probe).
-export([f/1, main/0]).
f(a) -> ok.
main() -> f(b).
