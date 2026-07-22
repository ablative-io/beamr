-module(if_probe).
-export([main_catch/0, h/1, id/1]).

%% id/1 routes the argument at runtime so erlc cannot constant-fold `h(b)`.
id(X) -> X.

%% A single-branch `if` with no matching guard raises error:if_clause.
h(X) ->
    if
        X =:= a -> ok
    end.

%% Catchability proof in LOADED BYTECODE: `catch error:if_clause` must match the
%% raised reason (BEAM's bare atom `if_clause`) and return `caught`.
main_catch() ->
    try h(id(b)) of
        Result -> Result
    catch
        error:if_clause -> caught
    end.
