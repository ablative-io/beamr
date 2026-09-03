defmodule T1String do
  def main do
    "Hello, Wörld" |> String.upcase() |> String.split(", ") |> Enum.join("-") |> String.length()
  end
end
