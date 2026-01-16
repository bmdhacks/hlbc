// Tests while loops with continue/break statements
// Bug: decompiler produces empty while loop bodies when continue is used
// Original: while (cond) { if (x) continue; body; }
// Decompiled: while (cond) { }  // Empty body!

class WhileContinue {
	static function skipEvens():Int {
		var sum = 0;
		var i = 0;
		while (i < 10) {
			i++;
			if (i % 2 == 0)
				continue;
			sum += i;
		}
		return sum; // 1+3+5+7+9 = 25
	}

	static function breakEarly():Int {
		var sum = 0;
		var i = 0;
		while (i < 100) {
			i++;
			sum += i;
			if (sum > 10)
				break;
		}
		return sum; // 1+2+3+4+5 = 15
	}

	static function nestedContinue():Int {
		var count = 0;
		var i = 0;
		while (i < 5) {
			i++;
			var j = 0;
			while (j < 5) {
				j++;
				if (j == 3)
					continue;
				count++;
			}
		}
		return count; // 5 * 4 = 20 (skips j=3 each time)
	}

	static function main() {
		Sys.println("skipEvens=" + skipEvens());
		Sys.println("breakEarly=" + breakEarly());
		Sys.println("nestedContinue=" + nestedContinue());
	}
}
