 we need to extend the generic type system in crates/data , especially classes,
 with the concept of computed properties, which are computed by an Expr.

 one example that has to work is eg:
 'my_prefix-' + stringify(self.other_field).

 so that has to be supported in expressions.

 research and suggest how to implement.
