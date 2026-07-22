-module(scan_raisers).
-export([main/1]).
%% Flags, per function: refutable heads (func_info fail edges) AND in-body
%% no-match raisers (case_end / badmatch / if_end / try_case_end).
main(Files) -> lists:foreach(fun scan/1, Files), halt(0).
scan(File) ->
    {beam_file, _Mod, _Exp, _Attr, _Info, Code} = beam_disasm:file(File),
    io:format("~n-- ~s (~p functions)~n", [File, length(Code)]),
    lists:foreach(fun({function, Name, Arity, Entry, Ins}) ->
        Pre = pre_label(Ins, Entry, none),
        Heads = length([I || I <- Ins, refs_label(I, Pre)]),
        Raisers = [element(1, I) || I <- Ins, is_tuple(I),
                   lists:member(element(1, I), [case_end, badmatch, try_case_end])]
                  ++ [if_end || I <- Ins, I =:= if_end],
        case {Heads, Raisers} of
            {0, []} -> ok;
            _ -> io:format("  ~p/~p: head_edges=~p raisers=~p~n", [Name, Arity, Heads, Raisers])
        end
    end, Code).
pre_label([{label, L} | Rest], Entry, _) when L =/= Entry -> pre_label(Rest, Entry, L);
pre_label([{label, Entry} | _], Entry, Last) -> Last;
pre_label([_ | Rest], Entry, Last) -> pre_label(Rest, Entry, Last);
pre_label([], _, Last) -> Last.
refs_label({func_info, _, _, _}, _) -> false;
refs_label({label, _}, _) -> false;
refs_label(I, L) when is_tuple(I) -> refs_f(tuple_to_list(I), L);
refs_label(_, _) -> false.
refs_f([], _) -> false;
refs_f([{f, L} | _], L) -> true;
refs_f([H | T], L) when is_list(H) -> refs_f(H, L) orelse refs_f(T, L);
refs_f([H | T], L) when is_tuple(H) -> refs_f(tuple_to_list(H), L) orelse refs_f(T, L);
refs_f([_ | T], L) -> refs_f(T, L).
