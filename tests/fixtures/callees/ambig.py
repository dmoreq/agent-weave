class A1:
    def m(self):
        return 1


class B1(A1):
    def m(self):
        return 2


class C1(A1):
    def m(self):
        return 3


class D1(B1, C1):
    pass


def call_m1():
    item = D1()
    return item.m()


class A2:
    def m(self):
        return 1


class B2(A2):
    def m(self):
        return 2


class C2(A2):
    def m(self):
        return 3


class D2(B2, C2):
    pass


def call_m2():
    item = D2()
    return item.m()


class A3:
    def m(self):
        return 1


class B3(A3):
    def m(self):
        return 2


class C3(A3):
    def m(self):
        return 3


class D3(B3, C3):
    pass


def call_m3():
    item = D3()
    return item.m()


class A4:
    def m(self):
        return 1


class B4(A4):
    def m(self):
        return 2


class C4(A4):
    def m(self):
        return 3


class D4(B4, C4):
    pass


def call_m4():
    item = D4()
    return item.m()


class A5:
    def m(self):
        return 1


class B5(A5):
    def m(self):
        return 2


class C5(A5):
    def m(self):
        return 3


class D5(B5, C5):
    pass


def call_m5():
    item = D5()
    return item.m()
