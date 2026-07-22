-module(if_probe).
-export([main/0, main_catch/0, h/1, id/1]).
id(Y) -> Y.
h(X) -> if X =:= a -> ok end.
main() -> h(id(b)).
main_catch() -> try h(id(b)) catch error:if_clause -> caught end.
