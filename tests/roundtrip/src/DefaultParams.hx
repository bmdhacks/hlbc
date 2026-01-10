class DefaultParams {
	static function greet(name:String = "World", excited:Bool = false):String {
		var msg = "Hello, " + name;
		if (excited)
			msg += "!";
		return msg;
	}

	static function calculate(a:Int, b:Int = 10, c:Int = 100):Int {
		return a + b * c;
	}

	static function main() {
		trace(greet());
		trace(greet("Alice"));
		trace(greet("Bob", true));

		trace("calc1=" + calculate(1));
		trace("calc2=" + calculate(1, 2));
		trace("calc3=" + calculate(1, 2, 3));
	}
}
