declare module "jest-axe" {
  interface AxeResults {
    violations: unknown[];
  }

  export function axe(element: Element): Promise<AxeResults>;
  export const toHaveNoViolations: {
    toHaveNoViolations(results: AxeResults): {
      pass: boolean;
      message(): string;
    };
  };
}

declare namespace jest {
  interface Matchers<R> {
    toHaveNoViolations(): R;
  }
}
