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
		Sys.println(greet());
		Sys.println(greet("Alice"));
		Sys.println(greet("Bob", true));

		Sys.println("calc1=" + calculate(1));
		Sys.println("calc2=" + calculate(1, 2));
		Sys.println("calc3=" + calculate(1, 2, 3));
	}
}
