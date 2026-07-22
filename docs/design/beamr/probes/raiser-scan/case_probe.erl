-module(case_probe).
-export([main/0, main_catch/0, g/1, id/1]).
id(Y) -> Y.
g(X) -> case X of a -> ok end.
main() -> g(id(b)).
main_catch() -> try g(id(b)) catch error:{case_clause, b} -> caught end.
