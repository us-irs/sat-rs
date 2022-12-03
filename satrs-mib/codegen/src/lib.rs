use proc_macro2::{Ident, TokenStream};
use quote::{format_ident, quote, ToTokens};
use syn::spanned::Spanned;
use syn::{parse_macro_input, AttributeArgs, Item, Lit, LitStr, Meta, NestedMeta, Type};

#[derive(Default)]
struct ResultExtGenerator {
    name_str: Option<Ident>,
    info_str: Option<LitStr>,
}

#[proc_macro_attribute]
pub fn resultcode(
    args: proc_macro::TokenStream,
    item: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    let args = parse_macro_input!(args as AttributeArgs);
    let input = parse_macro_input!(item as Item);
    let mut result_ext_generator = ResultExtGenerator::default();
    result_ext_generator.parse(args, input).into()
}

impl ResultExtGenerator {
    pub fn parse(&mut self, args: AttributeArgs, input: Item) -> TokenStream {
        let mut output = input.to_token_stream();
        if let Err(e) = self.parse_args(args) {
            output.extend(e.into_compile_error());
            return output;
        }
        match self.gen_ext_struct(input) {
            Ok(ts) => output.extend(ts),
            Err(e) => output.extend(e.into_compile_error()),
        }
        output
    }

    pub fn parse_args(&mut self, args: AttributeArgs) -> syn::Result<()> {
        for arg in args {
            if let NestedMeta::Meta(Meta::NameValue(nvm)) = arg {
                if let Some(path) = nvm.path.segments.first() {
                    if path.ident == "info" {
                        if let Lit::Str(str) = nvm.lit {
                            self.info_str = Some(str);
                        } else {
                            return Err(syn::Error::new(
                                nvm.lit.span(),
                                "Only literal strings are allowed as information",
                            ));
                        }
                    } else {
                        return Err(syn::Error::new(
                            path.span(),
                            format!("Unknown attribute argument name {}", path.ident),
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    pub fn gen_ext_struct(&mut self, input: Item) -> syn::Result<TokenStream> {
        if let Item::Const(const_item) = &input {
            self.name_str = Some(const_item.ident.clone());
            if let Type::Path(p) = &const_item.ty.as_ref() {
                let mut valid_type_found = false;
                for seg in &p.path.segments {
                    if seg.ident == "ResultU16" {
                        valid_type_found = true;
                    }
                }
                if !valid_type_found {
                    return Err(syn::Error::new(
                        p.span(),
                        "Can only be applied on items of type ResultU16",
                    ));
                }
            }
        } else {
            return Err(syn::Error::new(
                input.span(),
                "Only const items are allowed to be use with this attribute",
            ));
        }
        let result_code_name = self.name_str.to_owned().unwrap();
        let name_as_str = result_code_name.to_string();
        let gen_struct_name = format_ident!("{}_EXT", result_code_name);
        let info_str = if let Some(info_str) = &self.info_str {
            info_str.value()
        } else {
            String::from("")
        };
        // TODO: Group string
        let gen_struct = quote! {
            const #gen_struct_name: satrs_mib::res_code::ResultU16Info =
                satrs_mib::res_code::ResultU16Info::const_new(
                    #name_as_str,
                    &#result_code_name,
                    "",
                    #info_str
                );
        };
        Ok(gen_struct)
    }
}
