import sys, csv, math
from collections import Counter
inp, out = sys.argv[1], sys.argv[2]
name = inp.rsplit("/", 1)[-1]

def features(r):
    a, b, c = float(r["x1"]), float(r["x2"]), float(r["x3"])
    return [1.0, a, b, c, c * c, a * b]

def fit(rows):
    xs = [features(r) for r in rows]
    ys = [float(r["y"]) for r in rows]
    n = len(xs[0])
    m = [[sum(x[i] * x[j] for x in xs) for j in range(n)] + [sum(x[i] * y for x, y in zip(xs, ys))] for i in range(n)]
    for i in range(n):
        p = max(range(i, n), key=lambda r: abs(m[r][i]))
        m[i], m[p] = m[p], m[i]
        for r in range(n):
            if r != i:
                f = m[r][i] / m[i][i]
                m[r] = [a - f * b for a, b in zip(m[r], m[i])]
    return [m[i][n] / m[i][i] for i in range(n)]

with open(out, "w") as o:
    if name == "instances.txt":
        for line in open(inp):
            v = list(map(float, line.split()))
            pts = list(zip(v[0::2], v[1::2]))
            order, left = [0], set(range(1, len(pts)))
            while left:
                here = pts[order[-1]]
                nxt = min(left, key=lambda c: (math.dist(here, pts[c]), c))
                order.append(nxt)
                left.remove(nxt)
            o.write(" ".join(map(str, order)) + "\n")
    elif name == "inputs.csv":
        w = fit(list(csv.DictReader(open("data/train.csv"))))
        for r in csv.DictReader(open(inp)):
            o.write(f"{sum(a * b for a, b in zip(w, features(r)))}\n")
    else:
        sys.path.insert(0, "data")
        import oracle
        for q in open(inp):
            q = q.strip()
            votes = Counter(oracle.ask(q, attempt) for attempt in range(5))
            o.write(votes.most_common(1)[0][0] + "\n")
