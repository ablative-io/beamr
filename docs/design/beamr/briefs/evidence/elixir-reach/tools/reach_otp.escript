#!/usr/bin/env escript
%% reach_otp.escript <file.beam> — OTP-side instrument for the elixir-reach measurement.
%% Walks the raw "Code" chunk opcode by opcode using beam_opcodes:opname/1 (OTP's own table),
%% printing an opcode-number histogram; refuses loudly on a number the table does not know.
%% Also prints ImpT (beam_lib) and call_ext* call counts per extfunc (beam_disasm).
%% Output: line-oriented, tab-separated, one record type per line prefix.
-mode(compile).
-compile([nowarn_deprecated_catch]).
main([File]) ->
    {ok, {Mod, [{"Code", CodeBin}]}} = beam_lib:chunks(File, ["Code"]),
    <<SubSize:32, InstSet:32, OpMax:32, _Labels:32, _Funs:32, _/binary>> = CodeBin,
    <<_:32, _:SubSize/binary, Code/binary>> = CodeBin,
    io:format("module\t~s~ninstset\t~p~nopcode_max_header\t~p~n", [Mod, InstSet, OpMax]),
    case walk(Code, 0, #{}) of
        {ok, Hist} ->
            io:format("walk\tok\n"),
            lists:foreach(fun({Op, N}) -> {Name, Ar} = beam_opcodes:opname(Op),
                io:format("op\t~p\t~s/~p\t~p~n", [Op, Name, Ar, N]) end, lists:sort(maps:to_list(Hist)));
        {refused, Op, Off, Hist} ->
            io:format("walk\trefused\t~p\toffset\t~p~n", [Op, Off]),
            lists:foreach(fun({O, N}) -> {Name, Ar} = beam_opcodes:opname(O),
                io:format("op\t~p\t~s/~p\t~p~n", [O, Name, Ar, N]) end, lists:sort(maps:to_list(Hist)))
    end,
    {ok, {Mod, [{imports, Imps}]}} = beam_lib:chunks(File, [imports]),
    lists:foreach(fun({M, F, A}) -> io:format("impt\t~s:~s/~p~n", [M, F, A]) end, Imps),
    case catch beam_disasm:file(File) of
        {beam_file, _, _, _, _, Fns} ->
            Calls = lists:foldl(fun({function, _, _, _, Is}, Acc) -> count_calls(Is, Acc) end, #{}, Fns),
            io:format("disasm\tok\n"),
            lists:foreach(fun({{M, F, A}, N}) -> io:format("callext\t~s:~s/~p\t~p~n", [M, F, A, N]) end,
                          lists:sort(maps:to_list(Calls)));
        Err -> io:format("disasm\tfailed\t~p~n", [Err])
    end;
main(_) -> io:format("usage: reach_otp.escript <file.beam>~n"), halt(2).

count_calls(Is, Acc0) ->
    lists:foldl(fun(I, Acc) ->
        case I of
            {call_ext, _, {extfunc, M, F, A}} -> maps:update_with({M, F, A}, fun(N) -> N + 1 end, 1, Acc);
            {call_ext_only, _, {extfunc, M, F, A}} -> maps:update_with({M, F, A}, fun(N) -> N + 1 end, 1, Acc);
            {call_ext_last, _, {extfunc, M, F, A}, _} -> maps:update_with({M, F, A}, fun(N) -> N + 1 end, 1, Acc);
            _ -> Acc
        end end, Acc0, Is).

walk(<<>>, _Off, Hist) -> {ok, Hist};
walk(<<Op, Rest/binary>>, Off, Hist) ->
    case catch beam_opcodes:opname(Op) of
        {_Name, Arity} when is_integer(Arity) ->
            case os:getenv("REACH_OFFSETS") of false -> ok; _ -> io:format("off\t~p\t~p~n", [Off, Op]) end,
            Hist1 = maps:update_with(Op, fun(N) -> N + 1 end, 1, Hist),
            case catch skip_args(Arity, Rest) of
                {'EXIT', _} -> {refused, Op, Off, Hist1};
                Rest1 -> walk(Rest1, Off + 1 + (byte_size(Rest) - byte_size(Rest1)), Hist1)
            end;
        _ -> {refused, Op, Off, Hist}
    end.

skip_args(0, Bin) -> Bin;
skip_args(N, Bin) -> skip_args(N - 1, skip_term(Bin)).

%% Compact term encoding (OTP 29 beam_asm:encode/2 inverse), value discarded.
skip_term(<<B, Rest/binary>>) ->
    Tag = B band 7,
    case Tag of
        7 -> skip_ext(B bsr 4, Rest);
        _ -> skip_val(B, Rest)
    end.
skip_val(B, Rest) when B band 8 =:= 0 -> Rest;
skip_val(B, <<_, Rest/binary>>) when B band 16 =:= 0 -> Rest;
skip_val(B, Rest) ->
    case B bsr 5 of
        7 -> {N, Rest1} = read_u(Rest), <<_:(N + 9)/binary, Rest2/binary>> = Rest1, Rest2;
        K -> <<_:(K + 2)/binary, Rest1/binary>> = Rest, Rest1
    end.
read_u(<<B, Rest/binary>>) when B band 8 =:= 0 -> {B bsr 4, Rest};
read_u(<<B, C, Rest/binary>>) when B band 16 =:= 0 -> {((B bsr 5) bsl 8) bor C, Rest};
read_u(<<B, Rest/binary>>) ->
    K = (B bsr 5) + 2, <<V:(K * 8), Rest1/binary>> = Rest, {V, Rest1}.
skip_ext(1, Rest) -> {N, R1} = read_u(Rest), skip_args(N, R1);          % list
skip_ext(2, Rest) -> {_, R1} = read_u(Rest), R1;                          % {fr,N} float register (beam_asm.erl:786-787)
skip_ext(3, Rest) -> {N, R1} = read_u(Rest), skip_pairs(N, R1);          % alloc list
skip_ext(4, Rest) -> {_, R1} = read_u(Rest), R1;                         % literal
skip_ext(5, Rest) -> R1 = skip_term(Rest), {_, R2} = read_u(R1), R2.     % typed register
skip_pairs(0, R) -> R;
skip_pairs(N, R) -> {_, R1} = read_u(R), {_, R2} = read_u(R1), skip_pairs(N - 1, R2).
