export interface TypeModel {
  name: string;
  type: string;
  params?: TypeParameterModel[];
  optional?: boolean;
  docs?: string;
}
export interface TypeParameterModel {
  name: string;
  default?: string;
}
export interface CommandModel {
  name: string;
  symbol?: string;
  payload: TypeModel[];
  output: string;
  docs?: string;
}
export interface PackageModel {
  name: string;
  types: TypeModel[];
  commands: CommandModel[];
}
