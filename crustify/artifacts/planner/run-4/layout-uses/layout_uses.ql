/**
 * Layout uses of structs and unions other than field accesses (those are
 * t2/fa_with_root.csv): sizeof/alignof/offsetof on the type, values of the
 * type (locals, parameters, returns, globals, copies), pointer arithmetic
 * and indexing on a pointer to it, and by-value embedding in another struct.
 *
 * cols: type_name, type_def_file, user_kind (function|global|struct),
 *       user_name, user_file, use_kind, line
 */
import cpp
import identity

/** The struct/union `t` names by value, typedefs and cv stripped, arrays too. */
Class valueAgg(Type t) {
  result = embeddedTypeOf(t.getUnderlyingType()).getUnderlyingType() and
  (result instanceof Struct or result instanceof Union)
}

/** The struct/union a pointer type points to. */
Class pointeeAgg(Type t) {
  result = valueAgg(t.getUnspecifiedType().(PointerType).getBaseType())
}

predicate named(Class c, string n, string f) {
  ownerOf(c, n, f) and not isAnonNamed(n)
}

predicate fnUser(Element e, string kind, string name, string file) {
  exists(Function fn | fn = e.(Expr).getEnclosingFunction() |
    kind = "function" and name = fn.getName() and file = pathOf(fn.getFile()))
  or
  not exists(e.(Expr).getEnclosingFunction()) and
  exists(GlobalVariable g | g.getInitializer().getExpr().getAChild*() = e |
    kind = "global" and name = g.getName() and file = pathOf(g.getFile()))
}

from Class c, string tn, string tf, string uk, string un, string uf, string use, int line
where
  named(c, tn, tf) and
  (
    exists(SizeofTypeOperator op | valueAgg(op.getTypeOperand()) = c and fnUser(op, uk, un, uf) and use = "sizeof" and line = op.getLocation().getStartLine())
    or
    exists(SizeofExprOperator op | valueAgg(op.getExprOperand().getType()) = c and fnUser(op, uk, un, uf) and use = "sizeof" and line = op.getLocation().getStartLine())
    or
    exists(AlignofTypeOperator op | valueAgg(op.getTypeOperand()) = c and fnUser(op, uk, un, uf) and use = "alignof" and line = op.getLocation().getStartLine())
    or
    exists(BuiltInOperationBuiltInOffsetOf op | valueAgg(op.getAChild().(TypeName).getType()) = c and fnUser(op, uk, un, uf) and use = "offsetof" and line = op.getLocation().getStartLine())
    or
    exists(LocalVariable v | valueAgg(v.getType()) = c and uk = "function" and un = v.getFunction().getName() and uf = pathOf(v.getFunction().getFile()) and use = "local" and line = v.getLocation().getStartLine())
    or
    exists(Parameter p, Function fn | p = fn.getAParameter() and fn.hasDefinition() and valueAgg(p.getType()) = c and uk = "function" and un = fn.getName() and uf = pathOf(fn.getFile()) and use = "param" and line = p.getLocation().getStartLine())
    or
    exists(Function fn | fn.hasDefinition() and valueAgg(fn.getType()) = c and uk = "function" and un = fn.getName() and uf = pathOf(fn.getFile()) and use = "return" and line = fn.getLocation().getStartLine())
    or
    exists(GlobalVariable g | valueAgg(g.getType()) = c and g.hasDefinition() and uk = "global" and un = g.getName() and uf = pathOf(g.getDefinition().getFile()) and use = "global" and line = g.getLocation().getStartLine())
    or
    exists(AssignExpr a | valueAgg(a.getType()) = c and fnUser(a, uk, un, uf) and use = "copy" and line = a.getLocation().getStartLine())
    or
    exists(PointerDereferenceExpr d | valueAgg(d.getType()) = c and not d.getParent() instanceof AddressOfExpr and not d.getParent() instanceof FieldAccess and not d.getParent() instanceof SizeofExprOperator and fnUser(d, uk, un, uf) and use = "deref_value" and line = d.getLocation().getStartLine())
    or
    exists(PointerArithmeticOperation op | pointeeAgg(op.getAnOperand().getType()) = c and fnUser(op, uk, un, uf) and use = "ptr_arith" and line = op.getLocation().getStartLine())
    or
    exists(ArrayExpr ae | valueAgg(ae.getType()) = c and ae.getArrayBase().getUnspecifiedType() instanceof PointerType and fnUser(ae, uk, un, uf) and use = "index" and line = ae.getLocation().getStartLine())
    or
    exists(Field f, string on, string of | valueAgg(fieldType(f)) = c and named(f.getDeclaringType(), on, of) and uk = "struct" and un = on and uf = of and use = "embed" and line = f.getLocation().getStartLine())
  )
select tn, tf, uk, un, uf, use, line
