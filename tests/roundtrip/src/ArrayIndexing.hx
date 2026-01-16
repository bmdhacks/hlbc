// Tests array read/write operations
// Bug: decompiler uses wrong method names for array access
// Original: arr[i] = arr[j]
// Decompiled: arr.get_length(i)  // Wrong!

class ArrayIndexing {
	static function swap(arr:Array<Int>, i:Int, j:Int) {
		var tmp = arr[i];
		arr[i] = arr[j];
		arr[j] = tmp;
	}

	static function reverse(arr:Array<Int>) {
		var len = arr.length;
		var i = 0;
		while (i < len / 2) {
			swap(arr, i, len - 1 - i);
			i++;
		}
	}

	static function sum(arr:Array<Int>):Int {
		var total = 0;
		for (i in 0...arr.length) {
			total += arr[i];
		}
		return total;
	}

	static function setAll(arr:Array<Int>, val:Int) {
		for (i in 0...arr.length) {
			arr[i] = val;
		}
	}

	static function main() {
		var a = [1, 2, 3, 4, 5];

		// Test swap
		swap(a, 0, 4);
		Sys.println("swap a[0]=" + a[0]); // 5
		Sys.println("swap a[4]=" + a[4]); // 1

		// Test reverse
		var b = [10, 20, 30, 40];
		reverse(b);
		Sys.println("reverse b[0]=" + b[0]); // 40
		Sys.println("reverse b[3]=" + b[3]); // 10

		// Test sum
		var c = [1, 2, 3, 4, 5];
		Sys.println("sum=" + sum(c)); // 15

		// Test setAll
		setAll(c, 7);
		Sys.println("setAll c[2]=" + c[2]); // 7
	}
}
