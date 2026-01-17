class Base {
    public var value:Int;
    public function new(v:Int) {
        value = v;
    }
}

class TypeA extends Base {
    public function new() {
        super(1);
    }
}

class TypeB extends Base {
    public function new() {
        super(2);
    }
}

class TypeC extends Base {
    public function new() {
        super(3);
    }
}

class TypeD extends Base {
    public function new() {
        super(4);
    }
}

class TypeE extends Base {
    public function new() {
        super(5);
    }
}

class TypeF extends Base {
    public function new() {
        super(6);
    }
}

class TypeG extends Base {
    public function new() {
        super(7);
    }
}

class TypeH extends Base {
    public function new() {
        super(8);
    }
}

class TypeI extends Base {
    public function new() {
        super(9);
    }
}

class TypeJ extends Base {
    public function new() {
        super(10);
    }
}

class FactoryPattern {
    public static function create(id:String):Base {
        if (id == "a") return new TypeA();
        else if (id == "b") return new TypeB();
        else if (id == "c") return new TypeC();
        else if (id == "d") return new TypeD();
        else if (id == "e") return new TypeE();
        else if (id == "f") return new TypeF();
        else if (id == "g") return new TypeG();
        else if (id == "h") return new TypeH();
        else if (id == "i") return new TypeI();
        else if (id == "j") return new TypeJ();
        else return new Base(0);
    }

    public static function main() {
        trace(create("a").value);
        trace(create("e").value);
        trace(create("j").value);
        trace(create("?").value);
    }
}
