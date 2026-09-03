defmodule T1Struct do
  defstruct name: "x", n: 0
  def main do
    s = %T1Struct{name: "beamr", n: 3}
    case s do
      %T1Struct{n: n} when n > 2 -> {:big, n}
      %T1Struct{} -> :small
    end
  end
end
