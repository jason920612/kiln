// Prints java.lang.Math results as raw long bits, for kiln-javamath's agreement tests.
//
//   java JvmRef.java <count> > dump.txt
//   KILN_JVM_DUMP=dump.txt cargo test -p kiln-javamath --release jvm_dump -- --nocapture
//
// Lines: `<fn> <arg bits> [<arg2 bits>] <result bits>`. Inputs come from a fixed xorshift stream
// (several magnitude classes per function plus the argument shapes the game uses), so the same
// count always prints the same file. The functions are exactly the ones vanilla calls
// (`java.lang.Math`, never `StrictMath`).
public class JvmRef {
    static long s = 0x9E3779B97F4A7C15L;

    static long next() {
        s ^= s << 13;
        s ^= s >>> 7;
        s ^= s << 17;
        return s;
    }

    /** Uniform in [0, 1). */
    static double unit() {
        return (next() >>> 11) / (double) (1L << 53);
    }

    static double range(double lo, double hi) {
        return lo + (hi - lo) * unit();
    }

    /** A float-valued double in [lo, hi) (what `(double) floatExpr` produces). */
    static double f(double lo, double hi) {
        return (double) (float) range(lo, hi);
    }

    static java.io.PrintStream out = new java.io.PrintStream(new java.io.BufferedOutputStream(System.out, 1 << 20));

    static void p1(String name, double x, double r) {
        out.println(name + " " + Double.doubleToRawLongBits(x) + " " + Double.doubleToRawLongBits(r));
    }

    static void p2(String name, double x, double y, double r) {
        out.println(name + " " + Double.doubleToRawLongBits(x) + " " + Double.doubleToRawLongBits(y) + " " + Double.doubleToRawLongBits(r));
    }

    public static void main(String[] args) {
        int n = args.length > 0 ? Integer.parseInt(args[0]) : 100000;
        // The Mth.SIN table and the ASIN/COS tables.
        for (int i = 0; i < 65536 && i < n; i++) {
            double a = i / 10430.378350470453;
            p1("sin", a, Math.sin(a));
        }
        for (int i = 0; i < 257; i++) {
            double a = i / 256.0;
            double as = Math.asin(a);
            p1("asin", a, as);
            p1("cos", as, Math.cos(as));
        }
        for (int i = 0; i < n; i++) {
            double a;
            switch (i % 6) {
                case 0: a = range(-Math.PI * 2, Math.PI * 2); break;
                case 1: a = range(-100, 100); break;
                case 2: a = f(-720, 720); break;
                case 3: a = range(-1e-3, 1e-3); break;
                case 4: a = range(-1e6, 1e6); break;
                default: a = f(-10, 10);
            }
            p1("sin", a, Math.sin(a));
            p1("cos", a, Math.cos(a));
        }
        for (int i = 0; i < n; i++) {
            double y, x;
            switch (i % 4) {
                case 0: y = range(-10, 10); x = range(-10, 10); break;
                case 1: y = f(-1, 1); x = f(-1, 1); break;
                case 2: y = range(-1e-5, 1e-5); x = range(-1, 1); break;
                default: y = Math.floor(range(-50, 50)); x = Math.floor(range(-50, 50));
            }
            p2("atan2", y, x, Math.atan2(y, x));
        }
        for (int i = 0; i < n; i++) {
            double a = i % 3 == 0 ? range(-1, 1) : i % 3 == 1 ? f(-1, 1) : range(-1, 1) * range(0, 1) * range(0, 1);
            p1("asin", a, Math.asin(a));
            p1("acos", a, Math.acos(a));
        }
        for (int i = 0; i < n; i++) {
            double a;
            switch (i % 5) {
                case 0: a = range(0, 10); break;
                case 1: a = Math.exp(range(-700, 700)); break;
                case 2: a = f(0, 1000); break;
                case 3: a = 1 + range(-1e-3, 1e-3); break;
                default: a = Math.floor(range(1, 1e6));
            }
            p1("log", a, Math.log(a));
        }
        for (int i = 0; i < n; i++) {
            double a;
            switch (i % 3) {
                case 0: a = Math.floor(range(0, 1000)); break;
                case 1: a = range(0, 1e6); break;
                default: a = range(-0.9, 5);
            }
            p1("log1p", a, Math.log1p(a));
        }
        for (int i = 0; i < n; i++) {
            double b, e;
            switch (i % 8) {
                case 0: b = range(0, 10); e = range(-20, 20); break;
                case 1: b = 2.0; e = (double) (float) ((Math.floor(range(0, 25)) - 12) / 12.0); break;
                case 2: b = 2.0; e = range(-6, 6); break;
                case 3: b = f(0, 30); e = f(-3, 3); break;
                case 4: b = 0.3; e = Math.floor(range(1, 6)); break;
                case 5: b = Math.floor(range(1, 100)); e = Math.floor(range(-5, 8)); break;
                case 6: b = range(-10, 10); e = Math.floor(range(-6, 6)); break;
                default: b = 2.718281828459045; e = -range(0, 40) / 16.0;
            }
            p2("pow", b, e, Math.pow(b, e));
        }
        // Zeros, infinities, NaN and the other special arguments.
        double[] sp = {0.0, -0.0, 1.0, -1.0, 2.0, -2.0, 0.5, -0.5, 3.0, 10.0, 1e-310, 4.9e-324, 1.7976931348623157e308, Double.POSITIVE_INFINITY, Double.NEGATIVE_INFINITY, Double.NaN};
        for (double a : sp) {
            p1("sin", a, Math.sin(a));
            p1("cos", a, Math.cos(a));
            p1("asin", a, Math.asin(a));
            p1("acos", a, Math.acos(a));
            p1("log", a, Math.log(a));
            p1("log1p", a, Math.log1p(a));
            for (double b : sp) {
                p2("atan2", a, b, Math.atan2(a, b));
                p2("pow", a, b, Math.pow(a, b));
            }
        }
        out.flush();
    }
}
