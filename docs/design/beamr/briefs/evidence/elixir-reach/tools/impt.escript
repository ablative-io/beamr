#!/usr/bin/env escript
%% impt.escript <file.beam>... — one line per file: "<file>\t<imported module>" for each distinct module in ImpT.
-mode(compile).
main(Files) ->
    lists:foreach(fun(F) ->
        case beam_lib:chunks(F, [imports]) of
            {ok, {_, [{imports, Imps}]}} ->
                Ms = lists:usort([M || {M, _, _} <- Imps]),
                lists:foreach(fun(M) -> io:format("~s\t~s~n", [F, M]) end, Ms);
            Err -> io:format("~s\tERROR\t~p~n", [F, Err])
        end end, Files).
