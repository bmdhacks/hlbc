class Base {
	var name:String;

	public function new(n:String) {
		this.name = n;
	}

	public function greet():String {
		return "Hello from " + name;
	}
}

class Derived extends Base {
	var suffix:String;

	public function new(n:String, s:String) {
		super(n);
		this.suffix = s;
	}

	override public function greet():String {
		return super.greet() + suffix;
	}
}

class Inheritance {
	static function main() {
		var b = new Base("Base");
		Sys.println(b.greet());

		var d = new Derived("Test", "!");
		Sys.println(d.greet());
	}
}
