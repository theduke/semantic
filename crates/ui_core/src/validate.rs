use std::{borrow::Borrow, rc::Rc};

pub trait Validator<V> {
    fn validate(&self, value: &V) -> Result<(), Vec<String>>;

    fn boxed(self) -> Box<dyn Validator<V>>
    where
        Self: Sized + 'static,
    {
        Box::new(self)
    }

    fn and(self, other: impl Validator<V> + 'static) -> AndValidator<V>
    where
        Self: Sized + 'static,
    {
        AndValidator::new(self).and(other)
    }
}

pub struct StringRequired;

impl Validator<String> for StringRequired {
    fn validate(&self, value: &String) -> Result<(), Vec<String>> {
        if value.trim().is_empty() {
            Err(vec!["Field is required".into()])
        } else {
            Ok(())
        }
    }
}

pub struct StringUrl;

impl Validator<String> for StringUrl {
    fn validate(&self, value: &String) -> Result<(), Vec<String>> {
        match url::Url::parse(value) {
            Ok(_) => Ok(()),
            Err(err) => Err(vec![err.to_string()]),
        }
    }
}


impl<T, V> Validator<T> for Rc<V>
where
    V: Validator<T>,
{
    fn validate(&self, value: &T) -> Result<(), Vec<String>> {
        let borrow = self.borrow();
        let inner: &V = &*borrow;
        inner.validate(value)
    }
}

impl<T> Validator<T> for Box<dyn Validator<T>> {
    fn validate(&self, value: &T) -> Result<(), Vec<String>> {
        self.as_ref().validate(value)
    }
}

pub struct AndValidator<V> {
    validators: Vec<Box<dyn Validator<V>>>,
}

impl<V> Validator<V> for AndValidator<V> {
    fn validate(&self, value: &V) -> Result<(), Vec<String>> {
        let mut all_errors = Vec::new();
        for val in &self.validators {
            if let Err(errors) = val.validate(value) {
                all_errors.extend(errors);
            }
        }
        if all_errors.is_empty() {
            Ok(())
        } else {
            Err(all_errors)
        }
    }
}

impl<V> AndValidator<V> {
    pub fn new(val: impl Validator<V> + 'static) -> Self {
        Self {
            validators: vec![val.boxed()],
        }
    }

    pub fn and(mut self, other: impl Validator<V> + 'static) -> Self {
        self.validators.push(other.boxed());
        self
    }
}
