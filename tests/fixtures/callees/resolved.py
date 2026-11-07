def r1():
    return 1


def r2():
    return 2


def r3():
    return 3


def r4():
    return 4


def r5():
    return 5


def r6():
    return 6


def hub_a():
    return r1() + r2() + r3()


def hub_b():
    return r4() + r5()


def hub_c():
    return r1() + r4()


def hub_d():
    return r2() + r5() + r6()


def hub_e():
    return r1() + r6()


def chain1():
    return chain2()


def chain2():
    return chain3()


def chain3():
    return 0


def multi1():
    return r1() + r2() + chain1()


def multi2():
    return r3() + r4() + chain2()


def multi3():
    return r5() + r6() + chain3()
