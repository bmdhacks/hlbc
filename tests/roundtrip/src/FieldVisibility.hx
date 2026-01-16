// Tests public/private field modifiers
// Bug: decompiler loses visibility modifiers
// Original: public var field:Int;
// Decompiled: var field:Int;  // Lost 'public'

class FieldVisibility {
	public var publicField:Int = 1;
	private var privateField:Int = 2;
	var defaultField:Int = 3;

	public static var staticPublic:Int = 10;
	private static var staticPrivate:Int = 20;
	static var staticDefault:Int = 30;

	public function new() {}

	public function getPublic():Int {
		return publicField;
	}

	public function getPrivate():Int {
		return privateField;
	}

	public function getDefault():Int {
		return defaultField;
	}

	public function setAll(val:Int) {
		publicField = val;
		privateField = val + 1;
		defaultField = val + 2;
	}

	public static function getStaticPublic():Int {
		return staticPublic;
	}

	public static function getStaticPrivate():Int {
		return staticPrivate;
	}

	static function main() {
		var obj = new FieldVisibility();

		Sys.println("public=" + obj.getPublic());
		Sys.println("private=" + obj.getPrivate());
		Sys.println("default=" + obj.getDefault());

		obj.setAll(100);
		Sys.println("after setAll public=" + obj.getPublic());
		Sys.println("after setAll private=" + obj.getPrivate());
		Sys.println("after setAll default=" + obj.getDefault());

		Sys.println("staticPublic=" + getStaticPublic());
		Sys.println("staticPrivate=" + getStaticPrivate());
		Sys.println("staticDefault=" + staticDefault);
	}
}
