// Keep Inno's task model (including /TASKS and previous choices) as the source
// of truth, while presenting the format children in a collapsible four-column grid.
var
  TaskChecks: array of TNewCheckBox;
  FormatChecks: array of TNewCheckBox;
  FormatsButton: TNewButton;
  FormatsExpanded, SyncingTasks: Boolean;

procedure SyncTaskControls;
var
  I: Integer;
begin
  SyncingTasks := True;
  try
    for I := 0 to GetArrayLength(TaskChecks) - 1 do begin
      TaskChecks[I].Checked := WizardForm.TasksList.Checked[TaskChecks[I].Tag];
      TaskChecks[I].Enabled := WizardForm.TasksList.ItemEnabled[TaskChecks[I].Tag];
    end;
    for I := 0 to GetArrayLength(FormatChecks) - 1 do begin
      FormatChecks[I].Checked := WizardForm.TasksList.Checked[FormatChecks[I].Tag];
      FormatChecks[I].Enabled := WizardForm.TasksList.ItemEnabled[FormatChecks[I].Tag];
      FormatChecks[I].Visible := FormatsExpanded;
    end;
  finally
    SyncingTasks := False;
  end;
end;

procedure TaskControlClick(Sender: TObject);
var
  Box: TNewCheckBox;
begin
  if SyncingTasks then exit;
  Box := TNewCheckBox(Sender);
  if Box.Checked then
    WizardForm.TasksList.CheckItem(Box.Tag, coCheckWithChildren)
  else
    WizardForm.TasksList.CheckItem(Box.Tag, coUncheck);
  SyncTaskControls;
end;

procedure ToggleFormats(Sender: TObject);
begin
  FormatsExpanded := not FormatsExpanded;
  if FormatsExpanded then
    FormatsButton.Caption := 'Hide file formats'
  else
    FormatsButton.Caption := 'Show file formats';
  SyncTaskControls;
end;

procedure CreateTaskControls;
var
  I, N, Row, ColumnWidth: Integer;
  Caption: String;
  Box: TNewCheckBox;
begin
  ColumnWidth := WizardForm.TasksList.Width div 4;
  Row := 0;
  for I := 0 to WizardForm.TasksList.Items.Count - 1 do begin
    Caption := WizardForm.TasksList.ItemCaption[I];
    if (Caption = 'Create Start Menu shortcuts') or
       (Caption = 'Create a &desktop icon') or
       (Caption = 'Create a desktop icon') or
       (Caption = ExpandConstant('{cm:CreateDesktopIcon}')) or
       (Caption = 'Add EdolWave to Open with and Default apps') or
       ((Length(Caption) > 0) and (Caption[1] = '.')) then begin
      Box := TNewCheckBox.Create(WizardForm);
      Box.Parent := WizardForm.SelectTasksPage;
      Box.Tag := I;
      Box.Caption := Caption;
      Box.Height := ScaleY(20);
      Box.OnClick := @TaskControlClick;
      if Caption[1] = '.' then begin
        N := GetArrayLength(FormatChecks);
        SetArrayLength(FormatChecks, N + 1);
        FormatChecks[N] := Box;
        Box.Left := WizardForm.TasksList.Left + (N mod 4) * ColumnWidth;
        Box.Top := WizardForm.TasksList.Top + ScaleY(104 + (N div 4) * 22);
        Box.Width := ColumnWidth - ScaleX(6);
        Box.Visible := False;
      end else begin
        N := GetArrayLength(TaskChecks);
        SetArrayLength(TaskChecks, N + 1);
        TaskChecks[N] := Box;
        Box.Left := WizardForm.TasksList.Left;
        Box.Top := WizardForm.TasksList.Top + ScaleY(Row);
        Box.Width := WizardForm.TasksList.Width;
        Row := Row + 24;
        if N = 1 then Row := Row + 8;
      end;
    end;
  end;
  FormatsButton := TNewButton.Create(WizardForm);
  FormatsButton.Parent := WizardForm.SelectTasksPage;
  FormatsButton.Left := WizardForm.TasksList.Left + ScaleX(18);
  FormatsButton.Top := WizardForm.TasksList.Top + ScaleY(76);
  FormatsButton.Width := ScaleX(148);
  FormatsButton.Height := ScaleY(24);
  FormatsButton.Caption := 'Show file formats';
  FormatsButton.OnClick := @ToggleFormats;
  FormatsExpanded := False;
  WizardForm.TasksList.Visible := False;
  SyncTaskControls;
end;

procedure CurPageChanged(CurPageID: Integer);
begin
  if CurPageID = wpSelectTasks then begin
    if FormatsButton = nil then CreateTaskControls;
    SyncTaskControls;
  end;
end;

function UpdateReadyMemo(Space, NewLine, MemoUserInfoInfo, MemoDirInfo,
  MemoTypeInfo, MemoComponentsInfo, MemoGroupInfo, MemoTasksInfo: String): String;
var
  I: Integer;
  Tasks, Formats, Caption: String;
begin
  Tasks := '';
  Formats := '';
  for I := 0 to WizardForm.TasksList.Items.Count - 1 do begin
    Caption := WizardForm.TasksList.ItemCaption[I];
    if (Length(Caption) > 0) and WizardForm.TasksList.Checked[I] then begin
      if Caption[1] = '.' then begin
        if Formats <> '' then Formats := Formats + ', ';
        Formats := Formats + Caption;
      end else if WizardForm.TasksList.ItemLevel[I] = 0 then
        Tasks := Tasks + Space + Caption + NewLine;
    end;
  end;
  if Formats <> '' then Tasks := Tasks + Space + 'Formats: ' + Formats + NewLine;
  Result := '';
  if MemoUserInfoInfo <> '' then Result := Result + MemoUserInfoInfo + NewLine + NewLine;
  if MemoDirInfo <> '' then Result := Result + MemoDirInfo + NewLine + NewLine;
  if MemoTypeInfo <> '' then Result := Result + MemoTypeInfo + NewLine + NewLine;
  if MemoComponentsInfo <> '' then Result := Result + MemoComponentsInfo + NewLine + NewLine;
  if MemoGroupInfo <> '' then Result := Result + MemoGroupInfo + NewLine + NewLine;
  if Tasks <> '' then Result := Result + 'Additional tasks:' + NewLine + Tasks;
  WizardForm.ReadyMemo.WordWrap := True;
end;
