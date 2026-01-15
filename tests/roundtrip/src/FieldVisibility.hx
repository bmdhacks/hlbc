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

		trace("public=" + obj.getPublic());
		trace("private=" + obj.getPrivate());
		trace("default=" + obj.getDefault());

		obj.setAll(100);
		trace("after setAll public=" + obj.getPublic());
		trace("after setAll private=" + obj.getPrivate());
		trace("after setAll default=" + obj.getDefault());

		trace("staticPublic=" + getStaticPublic());
		trace("staticPrivate=" + getStaticPrivate());
		trace("staticDefault=" + staticDefault);
	}
}
