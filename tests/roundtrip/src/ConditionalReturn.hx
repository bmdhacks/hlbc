// Tests functions with multiple return paths
// Bug: decompiler may miss return paths or produce unreachable code
// Original: if (cond) return x; return y;
// Decompiled: if (true) { return x; } // Missing else return!

class ConditionalReturn {
	static function classify(n:Int):String {
		if (n < 0)
			return "negative";
		if (n == 0)
			return "zero";
		if (n < 10)
			return "small";
		return "large";
	}

	static function modReturn(a:Int, b:Int):Int {
		var r = a % b;
		if (r < 0)
			return r + b;
		return r;
	}

	static function abs(n:Int):Int {
		if (n >= 0)
			return n;
		return -n;
	}

	static function sign(n:Int):Int {
		if (n > 0)
			return 1;
		if (n < 0)
			return -1;
		return 0;
	}

	static function clamp(v:Int, lo:Int, hi:Int):Int {
		if (v < lo)
			return lo;
		if (v > hi)
			return hi;
		return v;
	}

	static function main() {
		trace("classify(-5)=" + classify(-5));
		trace("classify(0)=" + classify(0));
		trace("classify(5)=" + classify(5));
		trace("classify(100)=" + classify(100));

		trace("modReturn(-7,3)=" + modReturn(-7, 3));
		trace("modReturn(7,3)=" + modReturn(7, 3));

		trace("abs(-42)=" + abs(-42));
		trace("abs(42)=" + abs(42));

		trace("sign(-5)=" + sign(-5));
		trace("sign(0)=" + sign(0));
		trace("sign(5)=" + sign(5));

		trace("clamp(5,0,10)=" + clamp(5, 0, 10));
		trace("clamp(-5,0,10)=" + clamp(-5, 0, 10));
		trace("clamp(15,0,10)=" + clamp(15, 0, 10));
	}
}
