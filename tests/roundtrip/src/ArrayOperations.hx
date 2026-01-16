class ArrayOperations {
	static function main() {
		var arr = new Array<Int>();

		// Push operations
		arr.push(10);
		arr.push(20);
		arr.push(30);

		// Direct index access
		var first = arr[0];
		var last = arr[arr.length - 1];

		// Index assignment
		arr[1] = 25;

		// Pop
		var popped = arr.pop();

		Sys.println("first=" + first + " last=" + last + " popped=" + popped);
		Sys.println("remaining=" + arr.length);

		// Sum via index
		var sum = 0;
		var i = 0;
		while (i < arr.length) {
			sum += arr[i];
			i++;
		}
		Sys.println("sum=" + sum);
	}
}
