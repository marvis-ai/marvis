import { Field, Fieldset, Input, Label, Legend } from "@headlessui/react";
import { cn } from "@marvis-ai/ui";

export const Onboarding = (): React.JSX.Element => {
  return (
    <div className="w-full h-full">
      <Fieldset className="w-full">
        <Legend className="text-lg font-bold">Configure Your Marvis AI</Legend>
        <Field>
          <Label className="text-sm/6 font-medium text-white">
            Your API Key
          </Label>
          <Input
            className={cn(
              "mt-3 block w-full rounded-lg border-none bg-white/5 px-3 py-1.5 text-sm/6 text-white",
              "focus:not-data-focus:outline-none data-focus:outline-2 data-focus:-outline-offset-2 data-focus:outline-white/25",
            )}
          />
        </Field>
      </Fieldset>
    </div>
  );
};
