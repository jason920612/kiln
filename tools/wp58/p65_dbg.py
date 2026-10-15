from p65_b2 import *
X0, Y, Z0 = 6500, 200, 6500
for z in (Z0, Z0 + 4):
    print("unit z", z)
    for dy in range(5, -1, -1):
        print(Y + dy, [a.block(X0 + dx, Y + dy, z).replace("Block{minecraft:", "")[:34] for dx in (-1, 0, 1)])
