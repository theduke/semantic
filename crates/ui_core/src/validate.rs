use std::{borrow::Borrow, marker::PhantomData, rc::Rc};

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

pub struct PassingValidator<V> {
    _marker: PhantomData<V>,
}

impl<V> PassingValidator<V> {
    pub fn new() -> Self {
        Self {
            _marker: PhantomData,
        }
    }
}

impl<V> Validator<V> for PassingValidator<V> {
    fn validate(&self, _value: &V) -> Result<(), Vec<String>> {
        Ok(())
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

pub struct StringFloat;

impl Validator<String> for StringFloat {
    fn validate(&self, value: &String) -> Result<(), Vec<String>> {
        let _: f64 = value
            .parse()
            .map_err(|_err| vec![format!("Invalid number")])?;
        Ok(())
    }
}

pub struct StringDateTime;

impl Validator<String> for StringDateTime {
    fn validate(&self, value: &String) -> Result<(), Vec<String>> {
        chrono::NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M").map_err(|_| {
            vec!["Expected a valid date + time (format: 2020-10-10 HH::MM)".to_string()]
        })?;
        Ok(())
    }
}

pub struct ValidateStringUrl;

impl Validator<String> for ValidateStringUrl {
    fn validate(&self, value: &String) -> Result<(), Vec<String>> {
        url::Url::parse(value)
            .map(|_| ())
            .map_err(|e| vec![format!("Expected a valid url: {e}")])
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
