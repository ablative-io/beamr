-module(reach_lc).
-export([main/0]).
main() -> X = lists:sum([1,2,3]), Y = X * 2, {ok, Y}.
