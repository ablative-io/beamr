defmodule T1EnumMap do
  def main do
    m = %{a: 1, b: 2, c: 3}
    m |> Enum.map(fn {k, v} -> {k, v * 2} end) |> Map.new() |> Map.get(:b)
  end
end
