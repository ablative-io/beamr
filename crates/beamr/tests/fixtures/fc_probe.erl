-module(fc_probe).
-export([probe/1, caught/1]).

%% Single literal-pattern clause: probe(b) matches no clause, so reaching the
%% func_info prelude must raise error:function_clause (pre-fix it looped forever).
probe(a) -> ok.

%% Catchability proof in LOADED BYTECODE: a try/catch that names the class and
%% reason must observe error:function_clause and return the atom `caught`.
caught(X) ->
    try probe(X) of
        Result -> Result
    catch
        error:function_clause -> caught
    end.
