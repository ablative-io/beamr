-module(reach_opbase).
-export([main/0]).
main() -> X = lists:sum([1,2,3]), Y = X * 2, {ok, Y}.
