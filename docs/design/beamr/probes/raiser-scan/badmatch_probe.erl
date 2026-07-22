-module(badmatch_probe).
-export([main/0, main_catch/0, id/1]).
id(Y) -> Y.
main() -> a = id(b).
main_catch() -> try begin a = id(b) end catch error:{badmatch, b} -> caught end.
