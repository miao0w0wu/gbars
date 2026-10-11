import struct
import sys

MG = 0x8000

def load_mem(path):
    b = open(path, 'rb').read()
    return b[:MG], b[MG:MG+0x40000]

def diff_mem(a, b, base_name):
    ai, ae = load_mem(a)
    bi, be = load_mem(b)
    print(f"== {base_name} ==")
    for name, x, y, base in (("IWRAM", ai, bi, 0x03000000), ("EWRAM", ae, be, 0x02000000)):
        diffs = [i for i in range(min(len(x), len(y))) if x[i] != y[i]]
        print(f"  {name}: {len(diffs)} byte diffs")
        # group into runs
        runs = []
        for i in diffs:
            if runs and i == runs[-1][1] + 1:
                runs[-1][1] = i
            else:
                runs.append([i, i])
        for s, e in runs[:60]:
            seg = " ".join(f"{y[j]:02x}/{x[j]:02x}" for j in range(s, min(e+1, s+8)))
            print(f"    {base+s:08X}-{base+e:08X} (n={e-s+1}) mgba/ours: {seg}")

def load_writes(path):
    b = open(path, 'rb').read()
    n = len(b) // 16
    out = []
    for i in range(n):
        out.append(struct.unpack_from('<IIII', b, i*16))
    return out

def cmp_writes(mg_path, gb_path):
    mg = load_writes(mg_path)
    gb = load_writes(gb_path)
    print(f"== writes: mgba={len(mg)} gbars={len(gb)} ==")
    n = min(len(mg), len(gb))
    for i in range(n):
        m = mg[i]
        g = gb[i]
        if m != g:
            print(f"  first divergence at record {i}:")
            print(f"    mgba : pc={m[0]:08X} addr={m[1]:08X} size={m[2]} val={m[3]:08X}")
            print(f"    gbars: pc={g[0]:08X} addr={g[1]:08X} size={g[2]} val={g[3]:08X}")
            lo = max(0, i-8)
            print("  context:")
            for j in range(lo, min(n, i+4)):
                tag = "  <<<" if j == i else ""
                print(f"    [{j}] mgba pc={mg[j][0]:08X} a={mg[j][1]:08X} sz={mg[j][2]} v={mg[j][3]:08X} | gbars pc={gb[j][0]:08X} a={gb[j][1]:08X} sz={gb[j][2]} v={gb[j][3]:08X}{tag}")
            return
    print("  no divergence in common prefix")
    if len(mg) != len(gb):
        print(f"  length differs: mgba={len(mg)} gbars={len(gb)}")

if __name__ == '__main__':
    mode = sys.argv[1]
    if mode == 'mem':
        diff_mem(sys.argv[2], sys.argv[3], sys.argv[4] if len(sys.argv) > 4 else "mem")
    else:
        cmp_writes(sys.argv[2], sys.argv[3])